//! SQLite store for projects, columns, cards, links and activity.

use std::path::Path;

use muster_proto::{Card, CardStatus, Column, ColumnCategory, ErrorCode, Link, Message, Priority, Project};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::rank;
use crate::util::{err, internal, invalid, new_id, not_found, now_ms, Result};

const MIGRATIONS: &[&str] = &[
    // v1
    r#"
    CREATE TABLE projects (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        path TEXT NOT NULL,
        color TEXT NOT NULL,
        position INTEGER NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE columns (
        id TEXT PRIMARY KEY,
        project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        title TEXT NOT NULL,
        color TEXT NOT NULL,
        category TEXT NOT NULL,
        position INTEGER NOT NULL
    );
    CREATE INDEX columns_project ON columns(project_id, position);
    CREATE TABLE cards (
        id TEXT PRIMARY KEY,
        project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        column_id TEXT NOT NULL REFERENCES columns(id),
        rank TEXT NOT NULL,
        title TEXT NOT NULL,
        agent_id TEXT NOT NULL,
        cwd TEXT NOT NULL,
        branch TEXT,
        prompt TEXT,
        notes TEXT,
        priority TEXT,
        due_at INTEGER,
        agent_session_id TEXT,
        status TEXT NOT NULL,
        status_since INTEGER NOT NULL,
        turn_started_at INTEGER,
        last_activity_at INTEGER,
        unread INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX cards_column ON cards(column_id, rank);
    CREATE INDEX cards_project ON cards(project_id);
    CREATE TABLE links (
        card_a TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
        card_b TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
        PRIMARY KEY (card_a, card_b)
    );
    CREATE TABLE activity (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
        ts INTEGER NOT NULL,
        kind TEXT NOT NULL,
        payload TEXT
    );
    CREATE INDEX activity_card ON activity(card_id, ts);
    "#,
    // v2: agent integration (phase 3)
    r#"
    ALTER TABLE cards ADD COLUMN needs_you_reason TEXT;
    ALTER TABLE cards ADD COLUMN pending_approval TEXT;
    ALTER TABLE cards ADD COLUMN context TEXT;
    ALTER TABLE cards ADD COLUMN tracker TEXT;
    CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    "#,
    // v3: automatic titles, worktree setup
    r#"
    ALTER TABLE projects ADD COLUMN worktree_setup TEXT;
    ALTER TABLE cards ADD COLUMN title_source TEXT NOT NULL DEFAULT 'user';
    ALTER TABLE cards ADD COLUMN setup TEXT;
    "#,
    // v4: restore after reboot, messages between cards
    r#"
    ALTER TABLE cards ADD COLUMN restorable INTEGER NOT NULL DEFAULT 0;
    CREATE TABLE messages (
        id TEXT PRIMARY KEY,
        card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
        from_card_id TEXT,
        text TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        delivered_at INTEGER
    );
    CREATE INDEX messages_card ON messages(card_id, created_at);
    "#,
    // v5: per-card credentials
    r#"
    ALTER TABLE cards ADD COLUMN card_token_hash TEXT;
    CREATE INDEX cards_token ON cards(card_token_hash);
    "#,
];

/// Activity rows kept per card.
pub const ACTIVITY_KEEP: i64 = 1000;

/// Who set a card's title. Everything but `user` means `titleAuto`.
pub mod title_source {
    pub const USER: &str = "user";
    pub const PLACEHOLDER: &str = "placeholder";
    pub const PROMPT: &str = "prompt";
    pub const AGENT: &str = "agent";
}

pub const DEFAULT_COLUMNS: &[(&str, ColumnCategory)] = &[
    ("To Do", ColumnCategory::Unstarted),
    ("In Progress", ColumnCategory::Started),
    ("Done", ColumnCategory::Done),
];

const PALETTE: &[&str] = &["#5b8def", "#e5734a", "#46a758", "#9b6cd9", "#d6a21e", "#e0558a", "#2fa4a9"];

fn sql_err(e: rusqlite::Error) -> muster_proto::ErrorBody {
    internal(format!("sqlite: {e}"))
}

pub struct Store {
    conn: Connection,
}

/// Values for a new card. The store assigns id, rank and timestamps.
pub struct NewCard<'a> {
    pub project_id: &'a str,
    pub column_id: &'a str,
    pub title: &'a str,
    pub agent_id: &'a str,
    pub cwd: &'a str,
    pub branch: Option<&'a str>,
    pub prompt: Option<&'a str>,
    /// An existing agent session the card resumes when started.
    pub agent_session_id: Option<&'a str>,
    /// One of [`title_source`].
    pub title_source: &'a str,
}

/// Agent-derived card fields, written together by [`Store::save_agent_state`].
#[derive(Debug, Clone)]
pub struct AgentState {
    pub card_id: String,
    pub status: CardStatus,
    pub status_since: i64,
    pub turn_started_at: Option<i64>,
    pub last_activity_at: Option<i64>,
    pub agent_session_id: Option<String>,
    pub needs_you_reason: Option<String>,
    pub pending_approval: Option<muster_proto::PendingApproval>,
    pub context: Option<muster_proto::ContextInfo>,
    /// Opaque tracker state, restored after a restart.
    pub tracker: Option<String>,
}

#[derive(Default)]
pub struct CardPatch {
    pub title: Option<String>,
    pub prompt: Option<Option<String>>,
    pub notes: Option<Option<String>>,
    pub priority: Option<Option<Priority>>,
    pub due_at: Option<Option<i64>>,
    pub title_auto: Option<bool>,
}

const CARD_COLS: &str = "id, project_id, column_id, rank, title, agent_id, cwd, branch, prompt, notes, \
     priority, due_at, agent_session_id, status, status_since, turn_started_at, last_activity_at, \
     unread, created_at, updated_at, needs_you_reason, pending_approval, context, title_source, setup, restorable";

fn card_from_row(r: &Row) -> rusqlite::Result<Card> {
    let priority: Option<String> = r.get(10)?;
    let status: String = r.get(13)?;
    Ok(Card {
        id: r.get(0)?,
        project_id: r.get(1)?,
        column_id: r.get(2)?,
        rank: r.get(3)?,
        title: r.get(4)?,
        agent_id: r.get(5)?,
        cwd: r.get(6)?,
        branch: r.get(7)?,
        prompt: r.get(8)?,
        notes: r.get(9)?,
        priority: priority.as_deref().and_then(Priority::parse),
        due_at: r.get(11)?,
        agent_session_id: r.get(12)?,
        status: CardStatus::parse(&status).unwrap_or(CardStatus::Exited),
        status_since: r.get(14)?,
        turn_started_at: r.get(15)?,
        last_activity_at: r.get(16)?,
        unread: r.get::<_, i64>(17)? != 0,
        has_session: false,
        created_at: r.get(18)?,
        updated_at: r.get(19)?,
        needs_you_reason: r.get(20)?,
        pending_approval: r.get::<_, Option<String>>(21)?.and_then(|s| serde_json::from_str(&s).ok()),
        context: r.get::<_, Option<String>>(22)?.and_then(|s| serde_json::from_str(&s).ok()),
        title_auto: r.get::<_, String>(23)? != title_source::USER,
        setup: r.get::<_, Option<String>>(24)?.and_then(|s| serde_json::from_str(&s).ok()),
        restorable: r.get::<_, i64>(25)? != 0,
        queued_messages: vec![],
    })
}

fn message_from_row(r: &Row) -> rusqlite::Result<Message> {
    Ok(Message {
        id: r.get(0)?,
        card_id: r.get(1)?,
        from_card_id: r.get(2)?,
        text: r.get(3)?,
        created_at: r.get(4)?,
        delivered_at: r.get(5)?,
    })
}

fn project_from_row(r: &Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get(0)?,
        name: r.get(1)?,
        path: r.get(2)?,
        color: r.get(3)?,
        position: r.get(4)?,
        worktree_setup: r.get(5)?,
    })
}

fn column_from_row(r: &Row) -> rusqlite::Result<Column> {
    let cat: String = r.get(4)?;
    Ok(Column {
        id: r.get(0)?,
        project_id: r.get(1)?,
        title: r.get(2)?,
        color: r.get(3)?,
        category: ColumnCategory::parse(&cat).unwrap_or(ColumnCategory::Unstarted),
        position: r.get(5)?,
    })
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let conn = Connection::open(path).map_err(sql_err)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory().map_err(sql_err)?)
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; \
             PRAGMA busy_timeout=2000; PRAGMA cache_size=-512;",
        )
        .map_err(sql_err)?;
        let mut store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        let version: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(sql_err)?;
        if version as usize > MIGRATIONS.len() {
            return Err(err(
                ErrorCode::Internal,
                format!("database schema v{version} is newer than this daemon understands"),
            ));
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = self.conn.transaction().map_err(sql_err)?;
            tx.execute_batch(sql).map_err(sql_err)?;
            tx.pragma_update(None, "user_version", (i + 1) as i64).map_err(sql_err)?;
            tx.commit().map_err(sql_err)?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> i64 {
        self.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap_or(0)
    }

    // -- projects ----------------------------------------------------------

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let mut st = self
            .conn
            .prepare_cached("SELECT id, name, path, color, position, worktree_setup FROM projects ORDER BY position, created_at")
            .map_err(sql_err)?;
        let rows = st.query_map([], project_from_row).map_err(sql_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(sql_err)
    }

    pub fn get_project(&self, id: &str) -> Result<Project> {
        self.conn
            .query_row("SELECT id, name, path, color, position, worktree_setup FROM projects WHERE id = ?1", [id], project_from_row)
            .optional()
            .map_err(sql_err)?
            .ok_or_else(|| not_found("project", id))
    }

    /// Adds a project with the default columns. Returns the project and its
    /// columns.
    pub fn add_project(&mut self, path: &str, name: &str, color: Option<&str>) -> Result<(Project, Vec<Column>)> {
        let exists: Option<String> = self
            .conn
            .query_row("SELECT id FROM projects WHERE path = ?1", [path], |r| r.get(0))
            .optional()
            .map_err(sql_err)?;
        if exists.is_some() {
            return Err(err(ErrorCode::Conflict, format!("a project for {path} already exists")));
        }
        let count: i64 = self.conn.query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0)).map_err(sql_err)?;
        let color = color.map(str::to_string).unwrap_or_else(|| PALETTE[count as usize % PALETTE.len()].to_string());
        let project = Project {
            id: new_id(),
            name: name.to_string(),
            path: path.to_string(),
            color,
            position: count,
            worktree_setup: None,
        };
        let tx = self.conn.transaction().map_err(sql_err)?;
        tx.execute(
            "INSERT INTO projects (id, name, path, color, position, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![project.id, project.name, project.path, project.color, project.position, now_ms()],
        )
        .map_err(sql_err)?;
        let mut columns = Vec::new();
        for (i, (title, cat)) in DEFAULT_COLUMNS.iter().enumerate() {
            let c = Column {
                id: new_id(),
                project_id: project.id.clone(),
                title: title.to_string(),
                color: String::new(),
                category: *cat,
                position: i as i64,
            };
            tx.execute(
                "INSERT INTO columns (id, project_id, title, color, category, position) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![c.id, c.project_id, c.title, c.color, c.category.as_str(), c.position],
            )
            .map_err(sql_err)?;
            columns.push(c);
        }
        tx.commit().map_err(sql_err)?;
        Ok((project, columns))
    }

    /// Updates a project. A new `position` moves it to that index and
    /// renumbers the others; returns every project whose row changed.
    pub fn update_project(
        &mut self,
        id: &str,
        name: Option<&str>,
        color: Option<&str>,
        position: Option<i64>,
        worktree_setup: Option<Option<&str>>,
    ) -> Result<Vec<Project>> {
        let mut p = self.get_project(id)?;
        if let Some(w) = worktree_setup {
            p.worktree_setup = w.filter(|w| !w.trim().is_empty()).map(str::to_string);
        }
        if let Some(n) = name {
            if n.trim().is_empty() {
                return Err(invalid("name must not be empty"));
            }
            p.name = n.to_string();
        }
        if let Some(c) = color {
            p.color = c.to_string();
        }
        self.conn
            .execute(
                "UPDATE projects SET name = ?2, color = ?3, worktree_setup = ?4 WHERE id = ?1",
                params![p.id, p.name, p.color, p.worktree_setup],
            )
            .map_err(sql_err)?;
        let mut changed = vec![];
        if let Some(pos) = position {
            let ids: Vec<String> = self.list_projects()?.into_iter().map(|p| p.id).collect();
            for pid in reorder(&ids, id, pos) {
                self.conn
                    .execute("UPDATE projects SET position = ?2 WHERE id = ?1", params![pid.0, pid.1])
                    .map_err(sql_err)?;
                if pid.0 != id {
                    changed.push(self.get_project(&pid.0)?);
                }
            }
        }
        changed.insert(0, self.get_project(id)?);
        Ok(changed)
    }

    pub fn remove_project(&mut self, id: &str) -> Result<()> {
        let tx = self.conn.transaction().map_err(sql_err)?;
        tx.execute("DELETE FROM cards WHERE project_id = ?1", [id]).map_err(sql_err)?;
        tx.execute("DELETE FROM columns WHERE project_id = ?1", [id]).map_err(sql_err)?;
        let n = tx.execute("DELETE FROM projects WHERE id = ?1", [id]).map_err(sql_err)?;
        if n == 0 {
            return Err(not_found("project", id));
        }
        tx.commit().map_err(sql_err)
    }

    // -- columns -----------------------------------------------------------

    pub fn list_columns(&self, project_id: &str) -> Result<Vec<Column>> {
        let mut st = self
            .conn
            .prepare_cached(
                "SELECT id, project_id, title, color, category, position FROM columns \
                 WHERE project_id = ?1 ORDER BY position",
            )
            .map_err(sql_err)?;
        let rows = st.query_map([project_id], column_from_row).map_err(sql_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(sql_err)
    }

    pub fn get_column(&self, id: &str) -> Result<Column> {
        self.conn
            .query_row(
                "SELECT id, project_id, title, color, category, position FROM columns WHERE id = ?1",
                [id],
                column_from_row,
            )
            .optional()
            .map_err(sql_err)?
            .ok_or_else(|| not_found("column", id))
    }

    pub fn create_column(
        &mut self,
        project_id: &str,
        title: &str,
        color: Option<&str>,
        category: Option<ColumnCategory>,
    ) -> Result<Column> {
        self.get_project(project_id)?;
        if title.trim().is_empty() {
            return Err(invalid("title must not be empty"));
        }
        let position = self.list_columns(project_id)?.len() as i64;
        let c = Column {
            id: new_id(),
            project_id: project_id.to_string(),
            title: title.to_string(),
            color: color.unwrap_or("").to_string(),
            category: category.unwrap_or(ColumnCategory::Unstarted),
            position,
        };
        self.conn
            .execute(
                "INSERT INTO columns (id, project_id, title, color, category, position) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![c.id, c.project_id, c.title, c.color, c.category.as_str(), c.position],
            )
            .map_err(sql_err)?;
        Ok(c)
    }

    /// Returns every column whose row changed, the updated one first.
    pub fn update_column(
        &mut self,
        id: &str,
        title: Option<&str>,
        color: Option<&str>,
        category: Option<ColumnCategory>,
        position: Option<i64>,
    ) -> Result<Vec<Column>> {
        let mut c = self.get_column(id)?;
        if let Some(t) = title {
            if t.trim().is_empty() {
                return Err(invalid("title must not be empty"));
            }
            c.title = t.to_string();
        }
        if let Some(col) = color {
            c.color = col.to_string();
        }
        if let Some(cat) = category {
            c.category = cat;
        }
        self.conn
            .execute(
                "UPDATE columns SET title = ?2, color = ?3, category = ?4 WHERE id = ?1",
                params![c.id, c.title, c.color, c.category.as_str()],
            )
            .map_err(sql_err)?;
        let mut changed = vec![];
        if let Some(pos) = position {
            let ids: Vec<String> = self.list_columns(&c.project_id)?.into_iter().map(|c| c.id).collect();
            for (cid, p) in reorder(&ids, id, pos) {
                self.conn.execute("UPDATE columns SET position = ?2 WHERE id = ?1", params![cid, p]).map_err(sql_err)?;
                if cid != id {
                    changed.push(self.get_column(&cid)?);
                }
            }
        }
        changed.insert(0, self.get_column(id)?);
        Ok(changed)
    }

    /// Deletes a column, appending its cards to `move_to` (same project).
    /// Returns the moved cards and the columns whose position changed.
    pub fn delete_column(&mut self, id: &str, move_to: &str) -> Result<(Column, Vec<Card>, Vec<Column>)> {
        let col = self.get_column(id)?;
        let target = self.get_column(move_to)?;
        if id == move_to {
            return Err(invalid("moveCardsTo must be a different column"));
        }
        if target.project_id != col.project_id {
            return Err(invalid("moveCardsTo must be in the same project"));
        }
        let moving = self.cards_in_column(id)?;
        let mut last = self.cards_in_column(move_to)?.last().map(|c| c.rank.clone());
        let now = now_ms();
        let tx = self.conn.transaction().map_err(sql_err)?;
        let mut moved = vec![];
        for mut card in moving {
            let r = rank::between(last.as_deref(), None);
            tx.execute(
                "UPDATE cards SET column_id = ?2, rank = ?3, updated_at = ?4 WHERE id = ?1",
                params![card.id, move_to, r, now],
            )
            .map_err(sql_err)?;
            card.column_id = move_to.to_string();
            card.rank = r.clone();
            card.updated_at = now;
            last = Some(r);
            moved.push(card);
        }
        tx.execute("DELETE FROM columns WHERE id = ?1", [id]).map_err(sql_err)?;
        tx.commit().map_err(sql_err)?;
        let mut shifted = vec![];
        for (i, c) in self.list_columns(&col.project_id)?.into_iter().enumerate() {
            if c.position != i as i64 {
                self.conn.execute("UPDATE columns SET position = ?2 WHERE id = ?1", params![c.id, i as i64]).map_err(sql_err)?;
                shifted.push(Column { position: i as i64, ..c });
            }
        }
        Ok((col, moved, shifted))
    }

    // -- cards -------------------------------------------------------------

    pub fn get_card(&self, id: &str) -> Result<Card> {
        let mut c = self
            .conn
            .query_row(&format!("SELECT {CARD_COLS} FROM cards WHERE id = ?1"), [id], card_from_row)
            .optional()
            .map_err(sql_err)?
            .ok_or_else(|| not_found("card", id))?;
        c.queued_messages = self.queued_messages(id)?;
        Ok(c)
    }

    /// Fills `queued_messages` with one query.
    fn with_messages(&self, mut cards: Vec<Card>) -> Result<Vec<Card>> {
        let mut st = self
            .conn
            .prepare_cached(
                "SELECT id, card_id, from_card_id, text, created_at, delivered_at FROM messages \
                 WHERE delivered_at IS NULL ORDER BY created_at, rowid",
            )
            .map_err(sql_err)?;
        let msgs: Vec<Message> =
            st.query_map([], message_from_row).map_err(sql_err)?.collect::<rusqlite::Result<_>>().map_err(sql_err)?;
        if !msgs.is_empty() {
            for c in cards.iter_mut() {
                c.queued_messages = msgs.iter().filter(|m| m.card_id == c.id).cloned().collect();
            }
        }
        Ok(cards)
    }

    pub fn list_cards(&self, project_id: &str) -> Result<Vec<Card>> {
        let mut st = self
            .conn
            .prepare_cached(&format!("SELECT {CARD_COLS} FROM cards WHERE project_id = ?1 ORDER BY column_id, rank"))
            .map_err(sql_err)?;
        let rows = st.query_map([project_id], card_from_row).map_err(sql_err)?;
        let cards = rows.collect::<rusqlite::Result<_>>().map_err(sql_err)?;
        self.with_messages(cards)
    }

    pub fn all_cards(&self) -> Result<Vec<Card>> {
        let mut st = self.conn.prepare_cached(&format!("SELECT {CARD_COLS} FROM cards")).map_err(sql_err)?;
        let rows = st.query_map([], card_from_row).map_err(sql_err)?;
        let cards = rows.collect::<rusqlite::Result<_>>().map_err(sql_err)?;
        self.with_messages(cards)
    }

    pub fn cards_in_column(&self, column_id: &str) -> Result<Vec<Card>> {
        let mut st = self
            .conn
            .prepare_cached(&format!("SELECT {CARD_COLS} FROM cards WHERE column_id = ?1 ORDER BY rank, id"))
            .map_err(sql_err)?;
        let rows = st.query_map([column_id], card_from_row).map_err(sql_err)?;
        let cards = rows.collect::<rusqlite::Result<_>>().map_err(sql_err)?;
        self.with_messages(cards)
    }

    /// Creates a draft card at the end of its column.
    pub fn create_card(&mut self, n: NewCard) -> Result<Card> {
        let col = self.get_column(n.column_id)?;
        if col.project_id != n.project_id {
            return Err(invalid("column is not in that project"));
        }
        let last = self.cards_in_column(n.column_id)?.last().map(|c| c.rank.clone());
        let now = now_ms();
        let card = Card {
            id: new_id(),
            project_id: n.project_id.to_string(),
            column_id: n.column_id.to_string(),
            rank: rank::between(last.as_deref(), None),
            title: n.title.to_string(),
            agent_id: n.agent_id.to_string(),
            cwd: n.cwd.to_string(),
            branch: n.branch.map(str::to_string),
            prompt: n.prompt.map(str::to_string),
            notes: None,
            priority: None,
            due_at: None,
            agent_session_id: n.agent_session_id.map(str::to_string),
            status: CardStatus::Draft,
            status_since: now,
            turn_started_at: None,
            last_activity_at: None,
            unread: false,
            has_session: false,
            created_at: now,
            updated_at: now,
            needs_you_reason: None,
            pending_approval: None,
            context: None,
            title_auto: n.title_source != title_source::USER,
            setup: None,
            restorable: false,
            queued_messages: vec![],
        };
        self.conn
            .execute(
                &format!("INSERT INTO cards ({CARD_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,NULL,NULL,NULL,?21,NULL,0)"),
                params![
                    card.id, card.project_id, card.column_id, card.rank, card.title, card.agent_id, card.cwd,
                    card.branch, card.prompt, card.notes, None::<String>, card.due_at, card.agent_session_id,
                    card.status.as_str(), card.status_since, card.turn_started_at, card.last_activity_at,
                    0i64, card.created_at, card.updated_at, n.title_source
                ],
            )
            .map_err(sql_err)?;
        self.log(&card.id, "created", None)?;
        Ok(card)
    }

    pub fn update_card(&mut self, id: &str, patch: CardPatch) -> Result<Card> {
        let mut c = self.get_card(id)?;
        let mut source: Option<&str> = None;
        if patch.title_auto == Some(true) {
            source = Some(title_source::PLACEHOLDER);
        }
        if let Some(t) = patch.title {
            if t.trim().is_empty() {
                return Err(invalid("title must not be empty"));
            }
            c.title = t;
            source = Some(title_source::USER);
        } else if patch.title_auto == Some(false) {
            source = Some(title_source::USER);
        }
        if let Some(src) = source {
            // Handing the title back keeps an automatic title's progress.
            let keep = src == title_source::PLACEHOLDER && c.title_auto;
            if !keep {
                self.conn
                    .execute("UPDATE cards SET title_source = ?2 WHERE id = ?1", params![id, src])
                    .map_err(sql_err)?;
            }
            c.title_auto = src != title_source::USER;
        }
        if let Some(p) = patch.prompt {
            c.prompt = p;
        }
        if let Some(n) = patch.notes {
            c.notes = n;
        }
        if let Some(p) = patch.priority {
            c.priority = p;
        }
        if let Some(d) = patch.due_at {
            c.due_at = d;
        }
        c.updated_at = now_ms();
        self.conn
            .execute(
                "UPDATE cards SET title = ?2, prompt = ?3, notes = ?4, priority = ?5, due_at = ?6, updated_at = ?7 \
                 WHERE id = ?1",
                params![c.id, c.title, c.prompt, c.notes, c.priority.map(|p| p.as_str()), c.due_at, c.updated_at],
            )
            .map_err(sql_err)?;
        Ok(c)
    }

    /// Moves a card into `column_id`, immediately before the card `before`
    /// and/or after the card `after`; with neither it goes to the end.
    pub fn move_card(&mut self, id: &str, column_id: &str, before: Option<&str>, after: Option<&str>) -> Result<Card> {
        let mut card = self.get_card(id)?;
        let col = self.get_column(column_id)?;
        if col.project_id != card.project_id {
            return Err(invalid("cannot move a card to another project"));
        }
        if before == Some(id) || after == Some(id) {
            return Err(invalid("a card cannot be placed relative to itself"));
        }
        let others: Vec<Card> = self.cards_in_column(column_id)?.into_iter().filter(|c| c.id != id).collect();
        let index_of = |cid: &str| {
            others
                .iter()
                .position(|c| c.id == cid)
                .ok_or_else(|| err(ErrorCode::Invalid, format!("card {cid} is not in the target column")))
        };
        // Index in `others` at which the card is inserted.
        let pos = match (before, after) {
            (Some(b), Some(a)) => {
                let (bi, ai) = (index_of(b)?, index_of(a)?);
                if ai + 1 != bi {
                    return Err(invalid("`after` and `before` must be adjacent cards"));
                }
                bi
            }
            (Some(b), None) => index_of(b)?,
            (None, Some(a)) => index_of(a)? + 1,
            (None, None) => others.len(),
        };
        let lo = if pos == 0 { None } else { Some(others[pos - 1].rank.as_str()) };
        let hi = others.get(pos).map(|c| c.rank.as_str());
        let new_rank = match (lo, hi) {
            (Some(l), Some(h)) if l >= h => None, // duplicate ranks: renumber below
            _ => Some(rank::between(lo, hi)),
        };
        let now = now_ms();
        let tx = self.conn.transaction().map_err(sql_err)?;
        match new_rank {
            Some(r) => {
                tx.execute(
                    "UPDATE cards SET column_id = ?2, rank = ?3, updated_at = ?4 WHERE id = ?1",
                    params![id, column_id, r, now],
                )
                .map_err(sql_err)?;
            }
            None => {
                // Only reachable if two cards share a rank; rewrite the column.
                let mut order: Vec<&str> = others.iter().map(|c| c.id.as_str()).collect();
                order.insert(pos, id);
                for (cid, r) in order.iter().zip(rank::spread(order.len())) {
                    tx.execute(
                        "UPDATE cards SET column_id = ?2, rank = ?3, updated_at = ?4 WHERE id = ?1",
                        params![cid, column_id, r, now],
                    )
                    .map_err(sql_err)?;
                }
            }
        }
        tx.commit().map_err(sql_err)?;
        let from = std::mem::replace(&mut card.column_id, column_id.to_string());
        if from != column_id {
            self.log(id, "move", Some(&serde_json::json!({"from": from, "to": column_id}).to_string()))?;
        }
        self.get_card(id)
    }

    pub fn delete_card(&mut self, id: &str) -> Result<Card> {
        let c = self.get_card(id)?;
        self.conn.execute("DELETE FROM cards WHERE id = ?1", [id]).map_err(sql_err)?;
        Ok(c)
    }

    /// Sets the status (recording `status_since` when it changes) and
    /// returns the card.
    pub fn set_status(&mut self, id: &str, status: CardStatus) -> Result<Card> {
        let c = self.get_card(id)?;
        if c.status == status {
            return Ok(c);
        }
        let now = now_ms();
        let unread = match status {
            CardStatus::Done | CardStatus::NeedsYou => true,
            CardStatus::Idle => false,
            _ => c.unread,
        };
        let turn = if status == CardStatus::Running { Some(now) } else { c.turn_started_at };
        self.conn
            .execute(
                "UPDATE cards SET status = ?2, status_since = ?3, unread = ?4, turn_started_at = ?5, updated_at = ?3, \
                 needs_you_reason = CASE WHEN ?2 = 'needs_you' THEN needs_you_reason END, \
                 pending_approval = CASE WHEN ?2 = 'needs_you' THEN pending_approval END \
                 WHERE id = ?1",
                params![id, status.as_str(), now, unread as i64, turn],
            )
            .map_err(sql_err)?;
        self.log(id, "status", Some(&serde_json::json!({"from": c.status.as_str(), "to": status.as_str()}).to_string()))?;
        self.get_card(id)
    }

    pub fn mark_read(&mut self, id: &str) -> Result<Card> {
        let c = self.get_card(id)?;
        if c.status == CardStatus::Done {
            // Reading a finished turn makes the card idle.
            self.set_status(id, CardStatus::Idle)?;
        }
        self.conn
            .execute("UPDATE cards SET unread = 0, updated_at = ?2 WHERE id = ?1", params![id, now_ms()])
            .map_err(sql_err)?;
        self.get_card(id)
    }

    /// Writes the agent-derived fields of a card in one go. Logs a status change and
    /// keeps `unread` consistent with the status model.
    pub fn save_agent_state(&mut self, s: &AgentState) -> Result<Card> {
        let c = self.get_card(&s.card_id)?;
        let unread = match s.status {
            _ if s.status == c.status => c.unread,
            CardStatus::Done | CardStatus::NeedsYou => true,
            CardStatus::Idle => false,
            _ => c.unread,
        };
        self.conn
            .execute(
                "UPDATE cards SET status = ?2, status_since = ?3, turn_started_at = ?4, last_activity_at = ?5, \
                 agent_session_id = ?6, needs_you_reason = ?7, pending_approval = ?8, context = ?9, tracker = ?10, \
                 unread = ?11, updated_at = ?12 WHERE id = ?1",
                params![
                    s.card_id,
                    s.status.as_str(),
                    s.status_since,
                    s.turn_started_at,
                    s.last_activity_at,
                    s.agent_session_id,
                    s.needs_you_reason,
                    s.pending_approval.as_ref().map(|p| serde_json::to_string(p).unwrap()),
                    s.context.as_ref().map(|p| serde_json::to_string(p).unwrap()),
                    s.tracker,
                    unread as i64,
                    now_ms()
                ],
            )
            .map_err(sql_err)?;
        if c.status != s.status {
            self.log(
                &s.card_id,
                "status",
                Some(&serde_json::json!({"from": c.status.as_str(), "to": s.status.as_str()}).to_string()),
            )?;
        }
        self.get_card(&s.card_id)
    }

    /// Sets an automatic title from `source` (`prompt` or `agent`) if the card's title
    /// is still automatic and not already from a better source. Returns the card when it
    /// changed.
    pub fn set_auto_title(&mut self, id: &str, title: &str, source: &str) -> Result<Option<Card>> {
        let (current, title_now): (String, String) = self
            .conn
            .query_row("SELECT title_source, title FROM cards WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(sql_err)?
            .ok_or_else(|| not_found("card", id))?;
        let allowed = match source {
            title_source::PROMPT => current == title_source::PLACEHOLDER,
            title_source::AGENT => current == title_source::PLACEHOLDER || current == title_source::PROMPT || current == title_source::AGENT,
            _ => false,
        };
        if !allowed || title_now == title {
            return Ok(None);
        }
        self.conn
            .execute(
                "UPDATE cards SET title = ?2, title_source = ?3, updated_at = ?4 WHERE id = ?1",
                params![id, title, source, now_ms()],
            )
            .map_err(sql_err)?;
        self.get_card(id).map(Some)
    }

    pub fn set_setup(&mut self, id: &str, setup: Option<&muster_proto::SetupInfo>) -> Result<Card> {
        self.conn
            .execute(
                "UPDATE cards SET setup = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, setup.map(|s| serde_json::to_string(s).unwrap()), now_ms()],
            )
            .map_err(sql_err)?;
        self.get_card(id)
    }

    /// Marks the card `needs_you` with a reason (no tracker involved).
    pub fn set_needs_you(&mut self, id: &str, reason: &str) -> Result<Card> {
        self.set_status(id, CardStatus::NeedsYou)?;
        self.conn
            .execute("UPDATE cards SET needs_you_reason = ?2 WHERE id = ?1", params![id, reason])
            .map_err(sql_err)?;
        self.get_card(id)
    }

    /// Stores the hash of a card's session token (`None` revokes it).
    pub fn set_card_token_hash(&mut self, id: &str, hash: Option<&str>) -> Result<()> {
        self.conn
            .execute("UPDATE cards SET card_token_hash = ?2 WHERE id = ?1", params![id, hash])
            .map_err(sql_err)?;
        Ok(())
    }

    /// The card whose session token has this hash.
    pub fn card_for_token_hash(&self, hash: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT id FROM cards WHERE card_token_hash = ?1", [hash], |r| r.get(0))
            .optional()
            .map_err(sql_err)
    }

    /// Keeps the newest [`ACTIVITY_KEEP`] activity rows per card.
    pub fn prune_activity(&mut self) -> Result<usize> {
        self.conn
            .execute(
                "DELETE FROM activity WHERE id IN (SELECT id FROM (SELECT id, ROW_NUMBER() OVER \
                 (PARTITION BY card_id ORDER BY id DESC) AS rn FROM activity) WHERE rn > ?1)",
                [ACTIVITY_KEEP],
            )
            .map_err(sql_err)
    }

    pub fn set_restorable(&mut self, id: &str, restorable: bool) -> Result<()> {
        self.conn
            .execute("UPDATE cards SET restorable = ?2 WHERE id = ?1", params![id, restorable as i64])
            .map_err(sql_err)?;
        Ok(())
    }

    // -- links -------------------------------------------------------------

    fn link_key(a: &str, b: &str) -> Link {
        if a <= b {
            Link { a: a.into(), b: b.into() }
        } else {
            Link { a: b.into(), b: a.into() }
        }
    }

    /// Links two cards (undirected). Returns `None` if they were already linked.
    pub fn add_link(&mut self, a: &str, b: &str) -> Result<Option<Link>> {
        if a == b {
            return Err(invalid("a card cannot be linked to itself"));
        }
        self.get_card(a)?;
        self.get_card(b)?;
        let l = Self::link_key(a, b);
        let n = self
            .conn
            .execute("INSERT OR IGNORE INTO links (card_a, card_b) VALUES (?1, ?2)", params![l.a, l.b])
            .map_err(sql_err)?;
        Ok((n > 0).then_some(l))
    }

    pub fn remove_link(&mut self, a: &str, b: &str) -> Result<Option<Link>> {
        let l = Self::link_key(a, b);
        let n = self
            .conn
            .execute("DELETE FROM links WHERE card_a = ?1 AND card_b = ?2", params![l.a, l.b])
            .map_err(sql_err)?;
        Ok((n > 0).then_some(l))
    }

    pub fn is_linked(&self, a: &str, b: &str) -> Result<bool> {
        let l = Self::link_key(a, b);
        self.conn
            .query_row("SELECT 1 FROM links WHERE card_a = ?1 AND card_b = ?2", params![l.a, l.b], |_| Ok(()))
            .optional()
            .map(|o| o.is_some())
            .map_err(sql_err)
    }

    /// Ids of the cards linked to `id`.
    pub fn linked(&self, id: &str) -> Result<Vec<String>> {
        let mut st = self
            .conn
            .prepare_cached(
                "SELECT card_b FROM links WHERE card_a = ?1 UNION SELECT card_a FROM links WHERE card_b = ?1",
            )
            .map_err(sql_err)?;
        let rows = st.query_map([id], |r| r.get(0)).map_err(sql_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(sql_err)
    }

    /// Links with at least one card in the project.
    pub fn project_links(&self, project_id: &str) -> Result<Vec<Link>> {
        let mut st = self
            .conn
            .prepare_cached(
                "SELECT DISTINCT l.card_a, l.card_b FROM links l JOIN cards c ON c.id = l.card_a OR c.id = l.card_b \
                 WHERE c.project_id = ?1 ORDER BY l.card_a, l.card_b",
            )
            .map_err(sql_err)?;
        let rows = st.query_map([project_id], |r| Ok(Link { a: r.get(0)?, b: r.get(1)? })).map_err(sql_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(sql_err)
    }

    // -- messages ----------------------------------------------------------

    pub fn add_message(&mut self, card_id: &str, from: Option<&str>, text: &str) -> Result<Message> {
        self.get_card(card_id)?;
        let m = Message {
            id: new_id(),
            card_id: card_id.into(),
            from_card_id: from.map(str::to_string),
            text: text.into(),
            created_at: now_ms(),
            delivered_at: None,
        };
        self.conn
            .execute(
                "INSERT INTO messages (id, card_id, from_card_id, text, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![m.id, m.card_id, m.from_card_id, m.text, m.created_at],
            )
            .map_err(sql_err)?;
        Ok(m)
    }

    pub fn queued_messages(&self, card_id: &str) -> Result<Vec<Message>> {
        let mut st = self
            .conn
            .prepare_cached(
                "SELECT id, card_id, from_card_id, text, created_at, delivered_at FROM messages \
                 WHERE card_id = ?1 AND delivered_at IS NULL ORDER BY created_at, rowid",
            )
            .map_err(sql_err)?;
        let rows = st.query_map([card_id], message_from_row).map_err(sql_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(sql_err)
    }

    pub fn get_message(&self, id: &str) -> Result<Message> {
        self.conn
            .query_row(
                "SELECT id, card_id, from_card_id, text, created_at, delivered_at FROM messages WHERE id = ?1",
                [id],
                message_from_row,
            )
            .optional()
            .map_err(sql_err)?
            .ok_or_else(|| not_found("message", id))
    }

    /// Removes a queued message; delivered ones cannot be cancelled.
    pub fn cancel_message(&mut self, id: &str) -> Result<Message> {
        let m = self.get_message(id)?;
        if m.delivered_at.is_some() {
            return Err(err(ErrorCode::Conflict, "the message was already delivered"));
        }
        self.conn.execute("DELETE FROM messages WHERE id = ?1", [id]).map_err(sql_err)?;
        Ok(m)
    }

    pub fn mark_delivered(&mut self, id: &str) -> Result<()> {
        self.conn
            .execute("UPDATE messages SET delivered_at = ?2 WHERE id = ?1", params![id, now_ms()])
            .map_err(sql_err)?;
        Ok(())
    }

    /// The persisted agent tracker state (opaque JSON), if any.
    pub fn tracker(&self, id: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT tracker FROM cards WHERE id = ?1", [id], |r| r.get(0))
            .optional()
            .map_err(sql_err)
            .map(Option::flatten)
    }

    pub fn set_context(&mut self, id: &str, context: Option<&muster_proto::ContextInfo>) -> Result<()> {
        self.conn
            .execute(
                "UPDATE cards SET context = ?2 WHERE id = ?1",
                params![id, context.map(|c| serde_json::to_string(c).unwrap())],
            )
            .map_err(sql_err)?;
        Ok(())
    }

    pub fn set_agent_session_id(&mut self, id: &str, sid: Option<&str>) -> Result<()> {
        self.conn.execute("UPDATE cards SET agent_session_id = ?2 WHERE id = ?1", params![id, sid]).map_err(sql_err)?;
        Ok(())
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(sql_err)
    }

    pub fn meta_set(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(sql_err)?;
        Ok(())
    }

    pub fn touch_activity(&mut self, id: &str, ts: i64) -> Result<()> {
        self.conn.execute("UPDATE cards SET last_activity_at = ?2 WHERE id = ?1", params![id, ts]).map_err(sql_err)?;
        Ok(())
    }

    fn log(&self, card_id: &str, kind: &str, payload: Option<&str>) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO activity (card_id, ts, kind, payload) VALUES (?1, ?2, ?3, ?4)",
                params![card_id, now_ms(), kind, payload],
            )
            .map_err(sql_err)?;
        Ok(())
    }

    pub fn activity_count(&self, card_id: &str) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM activity WHERE card_id = ?1", [card_id], |r| r.get(0))
            .map_err(sql_err)
    }
}

/// Moves `id` to index `pos` (clamped) within `ids`; returns `(id, position)`
/// for every entry whose position changed.
fn reorder(ids: &[String], id: &str, pos: i64) -> Vec<(String, i64)> {
    let mut order: Vec<&String> = ids.iter().filter(|x| x.as_str() != id).collect();
    let pos = pos.clamp(0, order.len() as i64) as usize;
    let me = ids.iter().find(|x| x.as_str() == id);
    if let Some(me) = me {
        order.insert(pos, me);
    }
    order
        .iter()
        .enumerate()
        .filter(|(i, x)| ids.iter().position(|y| y == **x) != Some(*i))
        .map(|(i, x)| ((*x).clone(), i as i64))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_project() -> (Store, Project, Vec<Column>) {
        let mut s = Store::open_in_memory().unwrap();
        let (p, cols) = s.add_project("/tmp/x", "x", None).unwrap();
        (s, p, cols)
    }

    fn card(s: &mut Store, p: &Project, col: &Column, title: &str) -> Card {
        s.create_card(NewCard {
            project_id: &p.id,
            column_id: &col.id,
            title,
            agent_id: "shell",
            cwd: "/tmp",
            branch: None,
            prompt: None,
            agent_session_id: None,
            title_source: title_source::USER,
        })
        .unwrap()
    }

    fn titles(s: &Store, col: &Column) -> Vec<String> {
        s.cards_in_column(&col.id).unwrap().into_iter().map(|c| c.title).collect()
    }

    #[test]
    fn migrations_are_idempotent() {
        let dir = std::env::temp_dir().join(format!("muster-store-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.db");
        {
            let mut s = Store::open(&path).unwrap();
            assert_eq!(s.schema_version(), MIGRATIONS.len() as i64);
            s.add_project("/a", "a", None).unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.list_projects().unwrap().len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn project_defaults() {
        let (mut s, p, cols) = store_with_project();
        assert_eq!(cols.len(), 3);
        assert_eq!(s.list_columns(&p.id).unwrap(), cols);
        assert_eq!(cols[0].category, ColumnCategory::Unstarted);
        assert_eq!(cols[2].category, ColumnCategory::Done);
        assert_eq!(s.add_project("/tmp/x", "dup", None).unwrap_err().code, ErrorCode::Conflict);
        let (q, _) = s.add_project("/tmp/y", "y", Some("#000")).unwrap();
        assert_eq!(q.position, 1);
        let changed = s.update_project(&q.id, Some("yy"), None, Some(0), None).unwrap();
        assert_eq!(changed[0].name, "yy");
        assert_eq!(changed[0].position, 0);
        let ids: Vec<_> = s.list_projects().unwrap().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![q.id.clone(), p.id.clone()]);
        s.remove_project(&p.id).unwrap();
        assert_eq!(s.list_projects().unwrap().len(), 1);
        assert_eq!(s.get_project(&p.id).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn card_ordering_and_moves() {
        let (mut s, p, cols) = store_with_project();
        let a = card(&mut s, &p, &cols[0], "a");
        let b = card(&mut s, &p, &cols[0], "b");
        let c = card(&mut s, &p, &cols[0], "c");
        assert_eq!(titles(&s, &cols[0]), ["a", "b", "c"]);
        // c before a
        s.move_card(&c.id, &cols[0].id, Some(&a.id), None).unwrap();
        assert_eq!(titles(&s, &cols[0]), ["c", "a", "b"]);
        // c after b
        s.move_card(&c.id, &cols[0].id, None, Some(&b.id)).unwrap();
        assert_eq!(titles(&s, &cols[0]), ["a", "b", "c"]);
        // between a and b
        s.move_card(&c.id, &cols[0].id, Some(&b.id), Some(&a.id)).unwrap();
        assert_eq!(titles(&s, &cols[0]), ["a", "c", "b"]);
        // to another column, end
        let moved = s.move_card(&a.id, &cols[1].id, None, None).unwrap();
        assert_eq!(moved.column_id, cols[1].id);
        assert_eq!(titles(&s, &cols[0]), ["c", "b"]);
        assert_eq!(titles(&s, &cols[1]), ["a"]);
        assert_eq!(s.activity_count(&a.id).unwrap(), 2);
        // errors
        assert_eq!(s.move_card(&b.id, &cols[0].id, Some(&a.id), None).unwrap_err().code, ErrorCode::Invalid);
        assert_eq!(s.move_card(&b.id, &cols[0].id, Some(&b.id), None).unwrap_err().code, ErrorCode::Invalid);
        assert_eq!(
            s.move_card(&b.id, &cols[0].id, Some(&c.id), Some(&c.id)).unwrap_err().code,
            ErrorCode::Invalid
        );
    }

    #[test]
    fn many_moves_only_touch_one_row() {
        let (mut s, p, cols) = store_with_project();
        let ids: Vec<String> = (0..20).map(|i| card(&mut s, &p, &cols[0], &i.to_string()).id).collect();
        // Repeatedly move the last card to the front.
        for _ in 0..200 {
            let list = s.cards_in_column(&cols[0].id).unwrap();
            let last = list.last().unwrap().id.clone();
            let before_ranks: Vec<_> = list.iter().filter(|c| c.id != last).map(|c| c.rank.clone()).collect();
            s.move_card(&last, &cols[0].id, Some(&list[0].id), None).unwrap();
            let after: Vec<_> = s.cards_in_column(&cols[0].id).unwrap();
            assert_eq!(after[0].id, last);
            let after_ranks: Vec<_> = after.iter().skip(1).map(|c| c.rank.clone()).collect();
            assert_eq!(before_ranks, after_ranks);
        }
        assert_eq!(s.cards_in_column(&cols[0].id).unwrap().len(), ids.len());
    }

    #[test]
    fn duplicate_ranks_are_repaired() {
        let (mut s, p, cols) = store_with_project();
        let a = card(&mut s, &p, &cols[0], "a");
        let b = card(&mut s, &p, &cols[0], "b");
        let c = card(&mut s, &p, &cols[0], "c");
        s.conn.execute("UPDATE cards SET rank = 'V'", []).unwrap();
        s.move_card(&c.id, &cols[0].id, Some(&b.id), Some(&a.id)).unwrap_or_else(|_| {
            // a/b order among equal ranks is by id; try the other adjacency.
            s.move_card(&c.id, &cols[0].id, Some(&a.id), Some(&b.id)).unwrap()
        });
        let list = s.cards_in_column(&cols[0].id).unwrap();
        assert_eq!(list[1].id, c.id);
        for w in list.windows(2) {
            assert!(w[0].rank < w[1].rank);
        }
    }

    #[test]
    fn update_status_and_read() {
        let (mut s, p, cols) = store_with_project();
        let a = card(&mut s, &p, &cols[0], "a");
        assert_eq!(a.status, CardStatus::Draft);
        let u = s
            .update_card(
                &a.id,
                CardPatch { notes: Some(Some("n".into())), priority: Some(Some(Priority::High)), ..Default::default() },
            )
            .unwrap();
        assert_eq!(u.notes.as_deref(), Some("n"));
        assert_eq!(u.priority, Some(Priority::High));
        let u = s.update_card(&a.id, CardPatch { notes: Some(None), ..Default::default() }).unwrap();
        assert_eq!(u.notes, None);
        assert_eq!(u.priority, Some(Priority::High));
        let r = s.set_status(&a.id, CardStatus::Running).unwrap();
        assert!(r.turn_started_at.is_some());
        let d = s.set_status(&a.id, CardStatus::Done).unwrap();
        assert!(d.unread);
        let i = s.mark_read(&a.id).unwrap();
        assert_eq!(i.status, CardStatus::Idle);
        assert!(!i.unread);
    }

    #[test]
    fn activity_is_pruned() {
        let (mut s, p, cols) = store_with_project();
        let a = card(&mut s, &p, &cols[0], "a");
        for _ in 0..(ACTIVITY_KEEP + 50) {
            s.log(&a.id, "x", None).unwrap();
        }
        assert!(s.activity_count(&a.id).unwrap() > ACTIVITY_KEEP);
        s.prune_activity().unwrap();
        assert_eq!(s.activity_count(&a.id).unwrap(), ACTIVITY_KEEP);
    }

    #[test]
    fn card_tokens() {
        let (mut s, p, cols) = store_with_project();
        let a = card(&mut s, &p, &cols[0], "a");
        s.set_card_token_hash(&a.id, Some("h1")).unwrap();
        assert_eq!(s.card_for_token_hash("h1").unwrap(), Some(a.id.clone()));
        s.set_card_token_hash(&a.id, None).unwrap();
        assert_eq!(s.card_for_token_hash("h1").unwrap(), None);
    }

    #[test]
    fn links_and_messages() {
        let (mut s, p, cols) = store_with_project();
        let a = card(&mut s, &p, &cols[0], "a");
        let b = card(&mut s, &p, &cols[0], "b");
        let l = s.add_link(&b.id, &a.id).unwrap().unwrap();
        assert!(l.a < l.b);
        assert_eq!(s.add_link(&a.id, &b.id).unwrap(), None);
        assert!(s.is_linked(&a.id, &b.id).unwrap() && s.is_linked(&b.id, &a.id).unwrap());
        assert_eq!(s.linked(&a.id).unwrap(), vec![b.id.clone()]);
        assert_eq!(s.project_links(&p.id).unwrap().len(), 1);
        assert_eq!(s.add_link(&a.id, &a.id).unwrap_err().code, ErrorCode::Invalid);
        let m1 = s.add_message(&a.id, Some(&b.id), "hi").unwrap();
        let m2 = s.add_message(&a.id, None, "there").unwrap();
        assert_eq!(s.get_card(&a.id).unwrap().queued_messages.len(), 2);
        assert_eq!(s.list_cards(&p.id).unwrap().iter().find(|c| c.id == a.id).unwrap().queued_messages[0], m1);
        s.mark_delivered(&m1.id).unwrap();
        assert_eq!(s.cancel_message(&m1.id).unwrap_err().code, ErrorCode::Conflict);
        s.cancel_message(&m2.id).unwrap();
        assert!(s.get_card(&a.id).unwrap().queued_messages.is_empty());
        assert!(s.remove_link(&a.id, &b.id).unwrap().is_some());
        assert!(!s.is_linked(&a.id, &b.id).unwrap());
    }

    #[test]
    fn delete_column_moves_cards() {
        let (mut s, p, cols) = store_with_project();
        card(&mut s, &p, &cols[2], "z");
        card(&mut s, &p, &cols[1], "x");
        card(&mut s, &p, &cols[1], "y");
        let (_, moved, shifted) = s.delete_column(&cols[1].id, &cols[2].id).unwrap();
        assert_eq!(moved.len(), 2);
        assert_eq!(titles(&s, &cols[2]), ["z", "x", "y"]);
        assert_eq!(shifted.len(), 1);
        assert_eq!(s.list_columns(&p.id).unwrap().iter().map(|c| c.position).collect::<Vec<_>>(), [0, 1]);
        let nc = s.create_column(&p.id, "Review", None, None).unwrap();
        assert_eq!(nc.position, 2);
        let ch = s.update_column(&nc.id, None, None, None, Some(0)).unwrap();
        assert_eq!(ch.len(), 3);
        assert_eq!(s.list_columns(&p.id).unwrap()[0].id, nc.id);
    }
}
