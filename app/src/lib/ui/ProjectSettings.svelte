<script lang="ts">
  import { keyText } from "../os";
  import type { Id } from "../protocol/types";
  import { useApp } from "../state/context";
  import Dialog from "./Dialog.svelte";
  import Swatches from "./Swatches.svelte";
  import { colorOr } from "./colors";

  let { projectId }: { projectId: Id } = $props();
  const app = useApp();
  // svelte-ignore state_referenced_locally
  const p = app.projects.find((x) => x.id === projectId);

  let name = $state(p?.name ?? "");
  let color = $state(colorOr(p?.color, "#0a84ff"));
  let script = $state(p?.worktreeSetup ?? "");
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function save() {
    if (!p || busy) return;
    busy = true;
    error = null;
    try {
      await app.client.request("project.update", {
        id: p.id,
        name: name.trim() && name.trim() !== p.name ? name.trim() : undefined,
        color: color !== p.color ? color : undefined,
        worktreeSetup: script !== (p.worktreeSetup ?? "") ? (script.trim() ? script : null) : undefined,
      });
      app.projectSettings = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog label="Project settings" onclose={() => (app.projectSettings = null)} width={600}>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <form
    class="body"
    onsubmit={(e) => {
      e.preventDefault();
      void save();
    }}
    onkeydown={(e) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        void save();
      }
    }}
  >
    <h2>Project settings</h2>
    {#if p}
      <div class="path" title={p.path}>{p.path}</div>
      <label class="row"><span class="label">Name</span><input class="field" bind:value={name} data-autofocus /></label>
      <div class="row"><span class="label">Colour</span><Swatches value={color} onpick={(c) => (color = c)} /></div>

      <div class="section">
        <span class="label">Worktree setup</span>
        <p class="help">
          Runs in each new git worktree after it is created and before the agent starts, with your login shell.
          Its output appears in the card's terminal; if it fails, the card asks for you and the agent does not start.
        </p>
        <textarea
          class="field script"
          rows="9"
          spellcheck="false"
          autocomplete="off"
          placeholder={'npm ci\ncp "$MUSTER_PROJECT_PATH/.env" .env'}
          bind:value={script}
        ></textarea>
        <dl class="vars">
          <dt>MUSTER_PROJECT_PATH</dt><dd>the main checkout</dd>
          <dt>MUSTER_WORKTREE_PATH</dt><dd>the new worktree (also the working directory)</dd>
          <dt>MUSTER_BRANCH</dt><dd>the worktree's branch</dd>
        </dl>
      </div>

      {#if error}<div class="err">{error}</div>{/if}
      <footer>
        <span class="keys"><kbd>{keyText("⌘").replace("+", "")}</kbd><kbd>↵</kbd> save</span>
        <span class="spacer"></span>
        <button type="button" class="btn" onclick={() => (app.projectSettings = null)}>Cancel</button>
        <button type="submit" class="btn primary" disabled={busy}>Save</button>
      </footer>
    {:else}
      <p>This project no longer exists.</p>
    {/if}
  </form>
</Dialog>

<style>
  .body {
    padding: 16px 18px 14px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    overflow-y: auto;
  }
  h2 {
    margin: 0;
    font-size: 15px;
  }
  .path {
    margin-top: -8px;
    font: 11.5px var(--mono);
    color: var(--text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row {
    display: grid;
    grid-template-columns: 90px 1fr;
    align-items: center;
    gap: 10px;
  }
  .row :global(.sw) {
    padding: 0;
    display: flex;
    flex-wrap: wrap;
  }
  .section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .help {
    margin: 0;
    color: var(--text-2);
    font-size: 12px;
  }
  .script {
    font: 12px/1.5 var(--mono);
    resize: vertical;
    min-height: 120px;
    tab-size: 2;
  }
  .vars {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px 12px;
    margin: 0;
    font-size: 12px;
    color: var(--text-2);
  }
  dt {
    font: 11.5px var(--mono);
    color: var(--text);
    -webkit-user-select: text;
    user-select: text;
  }
  dd {
    margin: 0;
  }
  .err {
    color: var(--danger);
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .keys {
    color: var(--text-3);
    font-size: 11px;
  }
  .spacer {
    flex: 1;
  }
</style>
