<script lang="ts">
  import { useApp } from "../state/context";
  import Dialog from "./Dialog.svelte";

  const app = useApp();
  const GROUPS: Array<{ title: string; keys: Array<[string, string]> }> = [
    {
      title: "Everywhere",
      keys: [
        ["⌘T", "Board / Tracker"],
        ["⌘1 … ⌘9", "Switch project"],
        ["⌘,", "Settings"],
        ["⌘B", "Hide / show projects"],
        ["?", "This sheet"],
      ],
    },
    {
      title: "Board",
      keys: [
        ["← → ↑ ↓  or  H J K L", "Move the selection"],
        ["⇧ + arrows  or  ⇧H ⇧J ⇧K ⇧L", "Move the selected card"],
        ["↵", "Open the card"],
        ["N", "New session (then ↵ or 1–9 for the agent, M for more options)"],
        ["⇧N", "New card with all options"],
        ["S", "Start a draft"],
        ["Right-click a card", "Link, hand off, restore"],
      ],
    },
    {
      title: "Tracker",
      keys: [
        ["↑ ↓  or  J K", "Move the selection"],
        ["↵", "Open the card"],
      ],
    },
    {
      title: "Card window",
      keys: [
        ["Esc  or  ⌘W", "Close (Esc goes to the terminal while it has focus)"],
        ["⌃⇥", "Terminal / Chat"],
        ["⌃`", "Focus terminal / side panel"],
        ["⌘I", "Hide / show the details column"],
        ["⌘C  ⌘V", "Copy / paste in the terminal"],
        ["⌘K", "Clear the terminal"],
        ["⌘← ⌘→  ⌥← ⌥→", "Line start / end, word left / right"],
        ["⇧↵", "New line without sending (agents)"],
      ],
    },
    {
      title: "Dialogs and lists",
      keys: [
        ["⌘↵", "Start / save"],
        ["⇧⌘↵", "Save as draft"],
        ["↑ ↓  type a letter", "Pick in a list"],
      ],
    },
  ];
</script>

<Dialog label="Keyboard shortcuts" onclose={() => (app.shortcutsOpen = false)} width={620}>
  <div class="body">
    <h2>Keyboard shortcuts</h2>
    <div class="cols">
      {#each GROUPS as g (g.title)}
        <section>
          <div class="label">{g.title}</div>
          <dl>
            {#each g.keys as [k, what] (k)}
              <dt><kbd>{k}</kbd></dt>
              <dd>{what}</dd>
            {/each}
          </dl>
        </section>
      {/each}
    </div>
    <footer><button class="btn primary" onclick={() => (app.shortcutsOpen = false)} data-autofocus>Done</button></footer>
  </div>
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
  .cols {
    columns: 2 260px;
    column-gap: 24px;
  }
  section {
    break-inside: avoid;
    margin-bottom: 12px;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 10px;
    margin: 6px 0 0;
    font-size: 12px;
  }
  dt {
    white-space: nowrap;
  }
  dd {
    margin: 0;
    color: var(--text-2);
  }
  footer {
    display: flex;
    justify-content: flex-end;
  }
</style>
