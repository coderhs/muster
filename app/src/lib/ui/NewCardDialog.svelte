<script lang="ts">
  import { keyText } from "../os";

  const mod = keyText("⌘").replace("+", "");
  import Select from "./Select.svelte";
  import { CATEGORY_COLOR, colorOr } from "./colors";
  import type { Id } from "../protocol/types";
  import { useApp } from "../state/context";
  import Dialog from "./Dialog.svelte";
  import AgentMark from "./AgentMark.svelte";
  import Icon from "./Icon.svelte";

  let { columnId: initialColumn }: { columnId: Id | null } = $props();
  const app = useApp();

  const LAST_AGENT = "muster.lastAgent";
  const remembered = (() => {
    try {
      return localStorage.getItem(LAST_AGENT);
    } catch {
      return null;
    }
  })();

  const agents = $derived(app.agentsWithShell);
  // svelte-ignore state_referenced_locally
  let agentId = $state(app.lastAgent(app.currentProjectId) ?? remembered ?? "");
  let cwd = $state("");
  let title = $state("");
  let prompt = $state("");
  let worktree = $state(false);
  let branch = $state("");
  let branchTouched = $state(false);
  let resume = $state(false);
  let resumeId = $state("");
  // svelte-ignore state_referenced_locally
  let columnId = $state(initialColumn ?? "");
  let busy = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    // Default agent: last used if still installed, else first installed.
    if (!agents.some((a) => a.id === agentId && a.installed)) {
      agentId = agents.find((a) => a.installed && a.id !== "shell")?.id ?? agents.find((a) => a.installed)?.id ?? "shell";
    }
  });
  $effect(() => {
    if (!columnId || !app.columns.some((c) => c.id === columnId)) {
      columnId = (app.columns.find((c) => c.category === "unstarted") ?? app.columns[0])?.id ?? "";
    }
  });
  $effect(() => {
    if (!branchTouched) branch = slug(title || prompt.split("\n")[0] || "");
  });

  function slug(s: string): string {
    const base = s
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 40)
      .replace(/-+$/, "");
    return base ? `muster/${base}` : "";
  }

  const branchValid = $derived(!worktree || /^(?!\/)(?!.*\.\.)(?!.*\/\/)[A-Za-z0-9._\/-]+(?<![./])$/.test(branch));
  const isShell = $derived(agentId === "shell");
  const canResume = $derived(!isShell && agents.find((a) => a.id === agentId)?.capabilities?.resume !== false);
  const resuming = $derived(resume && canResume);
  const resumeValid = $derived(!resuming || /^[A-Za-z0-9_-]{1,128}$/.test(resumeId.trim()));
  $effect(() => {
    if (resuming) worktree = false;
  });
  // Title and prompt are optional: the daemon names untitled cards from the session.
  const canSubmit = $derived(!!columnId && !!agentId && branchValid && resumeValid && !busy);
  const setupScript = $derived(app.currentProject?.worktreeSetup?.trim() ? app.currentProject.worktreeSetup : null);

  async function submit(start: boolean) {
    if (!canSubmit || !app.currentProjectId) return;
    busy = true;
    error = null;
    try {
      localStorage.setItem(LAST_AGENT, agentId);
      localStorage.setItem(`muster.lastAgent.${app.currentProjectId}`, agentId);
    } catch {
      /* ignore */
    }
    try {
      const { card } = await app.client.request("card.create", {
        projectId: app.currentProjectId,
        columnId,
        agentId,
        title: title.trim() || undefined,
        prompt: resuming ? undefined : prompt.trim() || undefined,
        resumeSessionId: resuming ? resumeId.trim() : undefined,
        start,
        cwd: !worktree && cwd.trim() ? cwd.trim() : undefined,
        worktree: worktree ? { branch: branch.trim() } : undefined,
      });
      app.applyCard(card);
      app.selectedCardId = card.id;
      app.newCard = null;
      if (start) {
        app.focusTerminal = true;
        app.openCardId = card.id;
      }
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void submit(!e.shiftKey);
    }
  }
</script>

<Dialog label="New card" onclose={() => (app.newCard = null)} width={560}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="body" onkeydown={onKey}>
    <header>
      <h2>New card</h2>
      {#if app.currentProject}<span class="in">in {app.currentProject.name}</span>{/if}
      <span class="spacer"></span>
      <Select
        label="Column"
        compact
        value={columnId}
        options={app.columns.map((c) => ({ value: c.id, label: c.title, color: colorOr(c.color, CATEGORY_COLOR[c.category]) }))}
        onchange={(v) => (columnId = v)}
      />
    </header>

    <div class="agents" role="radiogroup" aria-label="Agent">
      {#each agents as a (a.id)}
        <button
          role="radio"
          aria-checked={agentId === a.id}
          class="agent"
          class:on={agentId === a.id}
          disabled={!a.installed}
          title={a.installed ? a.label : `${a.label} is not installed`}
          onclick={() => (agentId = a.id)}
        >
          <AgentMark agent={a} size={20} />
          <span>{a.label}</span>
        </button>
      {/each}
    </div>

    <input class="field title" placeholder="Title (optional; named from the session if empty)" bind:value={title} data-autofocus />
    <textarea
      class="field"
      rows="4"
      placeholder={isShell ? "Command to run (optional)" : "Initial prompt (optional)"}
      bind:value={prompt}
      disabled={resuming}
    ></textarea>

    <label class="cwd">
      <span>Folder</span>
      <input
        class="field mono"
        placeholder={app.currentProject?.path ?? "project folder"}
        bind:value={cwd}
        disabled={worktree}
        aria-label="Working directory"
        title={worktree ? "A new worktree sets the folder" : "Leave empty for the project folder"}
      />
    </label>

    <div class="wt">
      <label class="check">
        <input type="checkbox" bind:checked={worktree} disabled={resuming} />
        <Icon name="branch" size={14} /> New git worktree
      </label>
      {#if worktree}
        <input
          class="field mono"
          class:bad={!branchValid}
          placeholder="branch-name"
          bind:value={branch}
          oninput={() => (branchTouched = true)}
          aria-label="Branch name"
        />
      {/if}
    </div>
    {#if canResume}
      <div class="wt">
        <label class="check">
          <input type="checkbox" bind:checked={resume} />
          <Icon name="play" size={12} /> Resume an existing session
        </label>
        {#if resume}
          <input
            class="field mono"
            class:bad={!resumeValid && !!resumeId}
            placeholder="session id"
            bind:value={resumeId}
            aria-label="Session id to resume"
          />
        {/if}
      </div>
      {#if resume}
        <div class="setup-note">Folder must be the one the session was started in. The prompt is not sent.</div>
      {/if}
    {/if}
    {#if worktree && setupScript}
      <div class="setup-note">
        The project's worktree setup script runs in the new worktree before the agent starts.
        <button type="button" class="link" onclick={() => (app.projectSettings = app.currentProjectId)}>View script</button>
      </div>
    {/if}

    {#if error}<div class="err"><Icon name="warning" size={14} />{error}</div>{/if}

    <footer>
      <span class="keys"><kbd>{mod}</kbd><kbd>↵</kbd> start · <kbd>⇧</kbd><kbd>{mod}</kbd><kbd>↵</kbd> draft</span>
      <span class="spacer"></span>
      <button class="btn" onclick={() => (app.newCard = null)}>Cancel</button>
      <button class="btn" disabled={!canSubmit} onclick={() => submit(false)}>Save draft</button>
      <button class="btn primary" disabled={!canSubmit} onclick={() => submit(true)}>
        <Icon name="play" size={11} />Start now
      </button>
    </footer>
  </div>
</Dialog>

<style>
  .body {
    padding: 16px 18px 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
  }
  .in {
    color: var(--text-3);
  }
  .spacer {
    flex: 1;
  }
  .agents {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .agent {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 10px 0 5px;
    border-radius: 7px;
    border: 1px solid var(--border-strong);
    background: transparent;
  }
  .agent:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .agent.on {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .agent:disabled {
    opacity: 0.4;
  }
  .title {
    font-size: 14px;
    font-weight: 500;
  }
  .wt {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 28px;
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    color: var(--text-2);
  }
  .mono {
    font-family: var(--mono);
    font-size: 12px;
  }
  .bad {
    border-color: var(--danger);
  }
  .cwd {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--text-2);
  }
  .cwd input:disabled {
    opacity: 0.5;
  }
  .setup-note {
    font-size: 12px;
    color: var(--text-2);
    margin-top: -4px;
  }
  .link {
    border: 0;
    background: none;
    color: var(--accent);
    padding: 0;
    font-size: 12px;
  }
  .err {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--danger);
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 4px;
  }
  .keys {
    color: var(--text-3);
    font-size: 11px;
  }
</style>
