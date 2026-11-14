<script lang="ts">
  import { focusOnMount } from "./actions";
  import type { Project } from "../protocol/types";
  import { useApp } from "../state/context";
  import { errText } from "../protocol/client";
  import Icon from "./Icon.svelte";
  import Popover from "./Popover.svelte";
  import Swatches from "./Swatches.svelte";
  import Dialog from "./Dialog.svelte";
  import { colorOr } from "./colors";
  import { inkOn, railLabels } from "./rail";

  /** `rail`: the collapsed form, a narrow strip with one lettered tile per project. */
  let { rail = false }: { rail?: boolean } = $props();

  const app = useApp();
  const labels = $derived(railLabels(app.projects.map((p) => p.name)));

  let addingPath = $state<string | null>(null);
  let renaming = $state<string | null>(null);
  let renameDraft = $state("");
  let menu = $state<{ project: Project; anchor: HTMLElement } | null>(null);
  let removing = $state<Project | null>(null);

  async function add() {
    await app.requestAddProject();
  }
  // Path entry when there is no native folder picker (also requested from the empty state).
  let seenRequest = 0;
  $effect(() => {
    if (app.addProjectRequest !== seenRequest) {
      seenRequest = app.addProjectRequest;
      expand();
      addingPath = "";
    }
  });

  async function submitPath() {
    const p = (addingPath ?? "").trim();
    addingPath = null;
    if (p) await app.addProject(p).catch(() => {});
  }

  // Text entry needs the full width.
  function expand() {
    if (rail) app.toggleSidebar();
  }

  function startRename(p: Project) {
    menu = null;
    expand();
    renaming = p.id;
    renameDraft = p.name;
  }

  async function commitRename(p: Project) {
    const name = renameDraft.trim();
    renaming = null;
    if (name && name !== p.name) await app.call("project.update", { id: p.id, name }).catch(() => {});
  }

  async function move(p: Project, delta: number) {
    menu = null;
    const i = app.projects.findIndex((x) => x.id === p.id);
    await app.call("project.update", { id: p.id, position: i + delta }).catch(() => {});
  }

  async function remove(p: Project) {
    removing = null;
    await app.call("project.remove", { id: p.id }).catch(() => {});
  }

  function onKey(e: KeyboardEvent) {
    // Cmd+1..9 switches projects.
    if (e.metaKey && !e.shiftKey && !e.altKey && /^[1-9]$/.test(e.key)) {
      const p = app.projects[Number(e.key) - 1];
      if (p) {
        e.preventDefault();
        void app.selectProject(p.id);
      }
    }
  }
</script>

<svelte:window onkeydown={onKey} />

{#if rail}
  <nav class="sidebar rail" aria-label="Projects">
    <div class="titlebar" data-tauri-drag-region></div>
    <ul class="tiles">
      {#each app.projects as p, i (p.id)}
        {@const c = colorOr(p.color, "#0a84ff")}
        {@const needs = app.needsYouByProject.get(p.id)}
        <li>
          <button
            class="tile"
            class:active={p.id === app.currentProjectId}
            style="--c:{c};--ink:{inkOn(c)}"
            title="{p.name}{i < 9 ? ` (⌘${i + 1})` : ''}{needs ? ` · ${needs} need you` : ''}"
            aria-label={p.name}
            aria-current={p.id === app.currentProjectId ? "true" : undefined}
            onclick={() => {
              app.setView("board");
              void app.selectProject(p.id);
            }}
            oncontextmenu={(e) => {
              e.preventDefault();
              menu = { project: p, anchor: e.currentTarget as HTMLElement };
            }}
          >
            {labels[i]}
            {#if needs}<span class="tneeds"></span>{/if}
          </button>
        </li>
      {/each}
      <li><button class="icon-btn" title="Add project" onclick={add}><Icon name="plus" /></button></li>
    </ul>
    <div class="bottom">
      <button class="icon-btn" title="Settings (⌘,)" onclick={() => (app.settingsOpen = true)}><Icon name="gear" /></button>
    </div>
  </nav>
{:else}
<nav class="sidebar">
  <div class="titlebar" data-tauri-drag-region></div>
  <div class="section" data-tauri-drag-region>
    <span class="label">Projects</span>
    <button class="icon-btn" title="Add project" onclick={add}><Icon name="plus" /></button>
  </div>

  <ul class="list">
    {#each app.projects as p, i (p.id)}
      <li>
        {#if renaming === p.id}
                    <input
            class="field"
            bind:value={renameDraft}
            use:focusOnMount
            onblur={() => commitRename(p)}
            onkeydown={(e) => {
              if (e.key === "Enter") commitRename(p);
              if (e.key === "Escape") {
                e.stopPropagation();
                renaming = null;
              }
            }}
          />
        {:else}
          <button
            class="item"
            class:active={p.id === app.currentProjectId}
            onclick={() => {
              app.setView("board");
              void app.selectProject(p.id);
            }}
            ondblclick={() => startRename(p)}
            oncontextmenu={(e) => {
              e.preventDefault();
              menu = { project: p, anchor: e.currentTarget as HTMLElement };
            }}
            title={p.path}
          >
            <span class="pdot" style="--c:{colorOr(p.color, '#0a84ff')}"></span>
            <span class="name">{p.name}</span>
            {#if app.needsYouByProject.get(p.id)}
              <span class="needs" title="{app.needsYouByProject.get(p.id)} need you">{app.needsYouByProject.get(p.id)}</span>
            {/if}
            {#if i < 9}<span class="hint">⌘{i + 1}</span>{/if}
            <span
              class="more"
              role="button"
              tabindex="-1"
              title="Project options"
              onclick={(e) => {
                e.stopPropagation();
                menu = { project: p, anchor: e.currentTarget as HTMLElement };
              }}
              onkeydown={() => {}}><Icon name="more" size={14} /></span
            >
          </button>
        {/if}
      </li>
    {/each}
    {#if addingPath !== null}
      <li>
                <input
          class="field"
          placeholder="/path/to/project"
          bind:value={addingPath}
          use:focusOnMount
          onblur={submitPath}
          onkeydown={(e) => {
            if (e.key === "Enter") submitPath();
            if (e.key === "Escape") {
              e.stopPropagation();
              addingPath = null;
            }
          }}
        />
      </li>
    {/if}
  </ul>

  {#if !app.projects.length && addingPath === null && app.conn.kind === "ready"}
    <button class="first" onclick={add}><Icon name="folder" />Add a project folder</button>
  {/if}
  <div class="bottom">
    <button class="icon-btn" title="Settings (⌘,)" onclick={() => (app.settingsOpen = true)}><Icon name="gear" /></button>
  </div>
</nav>
{/if}

{#if menu}
  {@const p = menu.project}
  {@const idx = app.projects.findIndex((x) => x.id === p.id)}
  <Popover anchor={menu.anchor} onclose={() => (menu = null)} width={200}>
    <button class="mi" onclick={() => startRename(p)}><Icon name="pencil" size={14} />Rename</button>
    <button
      class="mi"
      onclick={() => {
        app.projectSettings = p.id;
        menu = null;
      }}><Icon name="folder" size={14} />Project settings…</button
    >
    <div class="sep"></div>
    <div class="sub">Colour</div>
    <Swatches
      value={p.color}
      onpick={(c) => {
        const id = p.id;
        menu = null;
        void app.call("project.update", { id, color: c }).catch(() => {});
      }}
    />
    <div class="sep"></div>
    <button class="mi" disabled={idx <= 0} onclick={() => move(p, -1)}>Move up</button>
    <button class="mi" disabled={idx >= app.projects.length - 1} onclick={() => move(p, 1)}>Move down</button>
    <div class="sep"></div>
    <button
      class="mi danger"
      onclick={() => {
        removing = p;
        menu = null;
      }}><Icon name="trash" size={14} />Remove project…</button
    >
  </Popover>
{/if}

{#if removing}
  {@const p = removing}
  <Dialog label="Remove project" onclose={() => (removing = null)} width={400}>
    <div class="confirm">
      <h3>Remove “{p.name}” from Muster?</h3>
      <p>Its board and cards are deleted. Files in <code>{p.path}</code> are not touched.</p>
      <div class="actions">
        <button class="btn" onclick={() => (removing = null)} data-autofocus>Cancel</button>
        <button class="btn primary" style="background:var(--danger)" onclick={() => remove(p)}>Remove</button>
      </div>
    </div>
  </Dialog>
{/if}

<style>
  .sidebar {
    width: 220px;
    flex: none;
    display: flex;
    flex-direction: column;
    background: var(--bg-sidebar);
    border-right: 1px solid var(--border);
    min-height: 0;
  }
  .sidebar.rail {
    width: 56px;
    align-items: center;
  }
  .tiles {
    list-style: none;
    margin: 0;
    padding: 4px 0;
    flex: 1;
    width: 100%;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
  }
  .tile {
    position: relative;
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border: 0;
    border-radius: 8px;
    background: var(--c);
    color: var(--ink);
    font-size: 13px;
    font-weight: 600;
    letter-spacing: -0.02em;
    opacity: 0.72;
  }
  .tile:hover {
    opacity: 1;
  }
  .tile.active {
    opacity: 1;
    box-shadow:
      0 0 0 2px var(--bg-sidebar),
      0 0 0 4px var(--c);
  }
  .tneeds {
    position: absolute;
    top: -3px;
    right: -3px;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--s-needs);
    border: 2px solid var(--bg-sidebar);
  }
  .rail .bottom {
    padding: 6px 0 10px;
  }
  .titlebar {
    height: var(--titlebar);
    flex: none;
  }
  .section {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 8px 4px 16px;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0 8px;
    overflow-y: auto;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 28px;
    padding: 0 6px 0 8px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    text-align: left;
  }
  .item:hover {
    background: var(--bg-hover);
  }
  .item.active {
    background: var(--bg-active);
    font-weight: 500;
  }
  .pdot {
    width: 10px;
    height: 10px;
    border-radius: 3px;
    background: var(--c);
    flex: none;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .needs {
    min-width: 18px;
    height: 16px;
    padding: 0 5px;
    border-radius: 8px;
    background: var(--s-needs);
    color: #1d1d1f;
    font-size: 10.5px;
    font-weight: 700;
    display: inline-grid;
    place-items: center;
  }
  .hint {
    font-size: 11px;
    color: var(--text-3);
  }
  .more {
    display: none;
    color: var(--text-2);
    width: 20px;
    height: 20px;
    place-items: center;
    border-radius: 4px;
  }
  .item:hover .more {
    display: inline-grid;
  }
  .item:hover .hint {
    display: none;
  }
  .more:hover {
    background: var(--bg-active);
  }
  li .field {
    height: 28px;
  }
  .bottom {
    padding: 6px 10px 10px;
    display: flex;
  }
  .first {
    margin: 8px;
    display: flex;
    align-items: center;
    gap: 8px;
    justify-content: center;
    height: 34px;
    border-radius: 8px;
    border: 1px dashed var(--border-strong);
    background: transparent;
    color: var(--text-2);
  }
  .confirm {
    padding: 18px 20px 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h3 {
    margin: 0;
    font-size: 14px;
  }
  p {
    margin: 0;
    color: var(--text-2);
  }
  code {
    font-family: var(--mono);
    font-size: 12px;
    word-break: break-all;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
</style>
