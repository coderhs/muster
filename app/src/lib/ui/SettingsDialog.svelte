<script lang="ts">
  import { onMount } from "svelte";
  import { useApp } from "../state/context";
  import Dialog from "./Dialog.svelte";
  import Select from "./Select.svelte";
  import { FONT_CHOICES, getTermPrefs, setTermPrefs } from "../term/prefs";

  const app = useApp();
  const s = $derived(app.settings);
  let notify = $state<boolean | null>(null);
  let notifyStatus = $state<string | null>(null);
  let island = $state<{ enabled: boolean; available: boolean } | null>(null);
  let term = $state(getTermPrefs());
  function setTerm(p: Partial<typeof term>) {
    term = { ...term, ...p };
    setTermPrefs(p);
  }
  const fontOptions = $derived(
    FONT_CHOICES.some((f) => f.value === term.fontFamily) ? FONT_CHOICES : [...FONT_CHOICES, { value: term.fontFamily, label: term.fontFamily }],
  );

  onMount(() => {
    app.platform.getNotify?.().then((v) => (notify = v)).catch(() => {});
    app.platform.notifyStatus?.().then((v) => (notifyStatus = v)).catch(() => {});
    app.platform.getIsland?.().then((v) => (island = v)).catch(() => {});
  });

  async function setSilence(n: number) {
    try {
      const { settings } = await app.call("settings.update", { silenceSeconds: Math.max(1, Math.min(600, n)) });
      app.settings = settings;
    } catch {
      /* toast shown */
    }
  }

  async function setDaemon(k: "keepAwake" | "autoRestore" | "clipboardPassthrough", v: boolean) {
    try {
      const { settings } = await app.call("settings.update", { [k]: v });
      app.settings = settings;
    } catch {
      /* toast shown */
    }
  }

  async function setIsland(v: boolean) {
    if (island) island = { ...island, enabled: v };
    await app.platform.setIsland?.(v).catch((e) => app.toast(`Could not save: ${e}`));
  }

  async function setNotify(v: boolean) {
    notify = v;
    await app.platform.setNotify?.(v).catch((e) => app.toast(`Could not save: ${e}`));
  }
</script>

<Dialog label="Settings" onclose={() => (app.settingsOpen = false)} width={520}>
  <div class="body">
    <h2>Settings</h2>

    <section>
      <div class="label">Daemon</div>
      {#if s}
        <label class="opt">
          <input type="checkbox" checked={s.keepAwake} onchange={(e) => setDaemon("keepAwake", (e.currentTarget as HTMLInputElement).checked)} data-autofocus />
          <span>
            <b>Keep the Mac awake while agents are running</b>
            <small>Prevents idle sleep only while a card is running; the display can still sleep. Released on battery below 10%.</small>
          </span>
        </label>
        <label class="opt">
          <input type="checkbox" checked={s.autoRestore} onchange={(e) => setDaemon("autoRestore", (e.currentTarget as HTMLInputElement).checked)} />
          <span>
            <b>Restore interrupted sessions automatically</b>
            <small>After a restart, cards whose sessions were cut off resume on their own instead of showing Restore.</small>
          </span>
        </label>
        {#if s.clipboardPassthrough != null}
          <label class="opt">
            <input type="checkbox" checked={s.clipboardPassthrough} onchange={(e) => setDaemon("clipboardPassthrough", (e.currentTarget as HTMLInputElement).checked)} />
            <span>
              <b>Let programs in cards copy to the Mac clipboard</b>
              <small>Lets terminal programs (editors, agents' copy commands) set the clipboard. Off by default: anything running in a card could otherwise replace what you copied.</small>
            </span>
          </label>
        {/if}
        {#if s.silenceSeconds != null}
          <div class="grid">
            <span>Idle after</span>
            <div class="stepper">
              <button class="btn small" aria-label="Shorter" onclick={() => setSilence(s.silenceSeconds! - (s.silenceSeconds! > 30 ? 10 : 1))}>−</button>
              <span class="num">{s.silenceSeconds} s</span>
              <button class="btn small" aria-label="Longer" onclick={() => setSilence(s.silenceSeconds! + (s.silenceSeconds! >= 30 ? 10 : 1))}>+</button>
              <small>of terminal silence, for agents without status hooks</small>
            </div>
          </div>
        {/if}
      {:else}
        <p class="muted">Not available from this daemon.</p>
      {/if}
    </section>

    <section>
      <div class="label">Notifications</div>
      {#if app.platform.setNotify && notify !== null}
        <label class="opt">
          <input type="checkbox" checked={notify} onchange={(e) => setNotify((e.currentTarget as HTMLInputElement).checked)} />
          <span>
            <b>Notify when a card needs you or finishes</b>
            <small>Only while the Muster window is not in front (also when only the menu-bar item is running). Clicking opens the card.</small>
          </span>
        </label>
        {#if notifyStatus === "denied"}
          <p class="warn">macOS is blocking Muster's notifications. Allow them in System Settings → Notifications → Muster.</p>
        {:else if notifyStatus === "unavailable"}
          <p class="warn">Notifications need the app bundle (Muster.app); this copy runs without one.</p>
        {:else if notifyStatus === "unknown"}
          <small>macOS will ask for permission the first time Muster has something to tell you.</small>
        {/if}
      {:else}
        <p class="muted">Notifications come from the desktop app.</p>
      {/if}
      {#if app.platform.setIsland && island}
        <label class="opt">
          <input type="checkbox" checked={island.enabled} onchange={(e) => setIsland((e.currentTarget as HTMLInputElement).checked)} />
          <span>
            <b>Show working and finished cards around the notch</b>
            <small>Right of the camera notch: cards that are working (orange), cards that need you (red) and the last three cards that finished (green tick). Point at the strip to see the cards by name. Clicking a name opens that card; a finished card stays until you click it there or it starts working again.</small>
          </span>
        </label>
        {#if !island.available}
          <small>No connected display has a notch, so there is nothing to show right now.</small>
        {/if}
      {/if}
    </section>

    <section>
      <div class="label">Terminal</div>
      <div class="grid">
        <span>Font</span>
        <Select label="Terminal font" value={term.fontFamily} options={fontOptions} onchange={(v) => setTerm({ fontFamily: v })} />
        <span>Weight</span>
        <Select
          label="Terminal font weight"
          value={term.bold ? "bold" : "regular"}
          options={[{ value: "regular", label: "Regular" }, { value: "bold", label: "Bold" }]}
          onchange={(v) => setTerm({ bold: v === "bold" })}
        />
        <span>Size</span>
        <div class="stepper">
          <button class="btn small" aria-label="Smaller" onclick={() => setTerm({ fontSize: Math.max(9, term.fontSize - 1) })}>−</button>
          <span class="num">{term.fontSize} pt</span>
          <button class="btn small" aria-label="Larger" onclick={() => setTerm({ fontSize: Math.min(24, term.fontSize + 1) })}>+</button>
        </div>
        <span>Line height</span>
        <div class="stepper">
          <button class="btn small" aria-label="Tighter" onclick={() => setTerm({ lineHeight: Math.max(1, Math.round((term.lineHeight - 0.05) * 100) / 100) })}>−</button>
          <span class="num">{term.lineHeight.toFixed(2)}</span>
          <button class="btn small" aria-label="Looser" onclick={() => setTerm({ lineHeight: Math.min(1.6, Math.round((term.lineHeight + 0.05) * 100) / 100) })}>+</button>
        </div>
      </div>
    </section>

    <footer><button class="btn primary" onclick={() => (app.settingsOpen = false)}>Done</button></footer>
  </div>
</Dialog>

<style>
  .body {
    padding: 16px 18px 14px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .opt {
    display: flex;
    gap: 10px;
    align-items: flex-start;
  }
  .opt input {
    margin-top: 3px;
  }
  .opt span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .opt b {
    font-weight: 500;
  }
  small,
  .muted {
    color: var(--text-2);
    font-size: 12px;
    margin: 0;
  }
  .warn {
    color: var(--s-needs);
    font-size: 12px;
    margin: 0;
  }
  .grid {
    display: grid;
    grid-template-columns: 90px 1fr;
    gap: 8px 10px;
    align-items: center;
  }
  .stepper {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .num {
    min-width: 44px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  footer {
    display: flex;
    justify-content: flex-end;
  }
</style>
