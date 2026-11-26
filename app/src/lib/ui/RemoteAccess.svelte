<script lang="ts">
  import { computer } from "../os";
  import { onDestroy, onMount } from "svelte";
  import { useApp } from "../state/context";
  import type { RemoteStatus } from "../protocol/types";
  import { agoLabel } from "./time";

  const app = useApp();
  let status = $state<RemoteStatus | null>(null);
  let busy = $state(false);
  let pairing = $state<{ url: string; expiresAt: number; svg: string } | null>(null);
  let now = $state(Date.now());
  let off: (() => void) | null = null;
  let timer: ReturnType<typeof setInterval> | null = null;

  onMount(() => {
    app.call("remote.status", {}, true).then((s) => (status = s)).catch(() => {});
    off = app.client.on("remote.changed", (s) => {
      // A newly paired device means the code on screen was used.
      if (pairing && status && s.devices.length > status.devices.length) pairing = null;
      status = s;
    });
    timer = setInterval(() => {
      now = Date.now();
      if (pairing && now > pairing.expiresAt) pairing = null;
    }, 1000);
  });
  onDestroy(() => {
    off?.();
    if (timer) clearInterval(timer);
  });

  async function setEnabled(on: boolean) {
    busy = true;
    try {
      status = await app.call("remote.set", { enabled: on });
      if (!on) pairing = null;
    } catch {
      /* toast shown */
    } finally {
      busy = false;
    }
  }

  async function pair() {
    try {
      const { url, expiresAt } = await app.call("remote.pair", {});
      const { default: qrcode } = await import("qrcode-generator");
      const qr = qrcode(0, "M");
      qr.addData(url);
      qr.make();
      pairing = { url, expiresAt, svg: qr.createSvgTag({ cellSize: 4, margin: 2, scalable: true }) };
    } catch {
      /* toast shown */
    }
  }

  async function revoke(id: string, name: string) {
    if (!confirm(`Unpair ${name}? It is disconnected at once and needs a new pairing link to come back.`)) return;
    try {
      status = await app.call("remote.revoke", { id });
    } catch {
      /* toast shown */
    }
  }

  function copy(text: string) {
    navigator.clipboard.writeText(text).then(
      () => app.toast("Copied", "info"),
      () => app.toast("Could not copy"),
    );
  }

  const minutesLeft = $derived(pairing ? Math.max(0, Math.ceil((pairing.expiresAt - now) / 60000)) : 0);
</script>

<section>
  <div class="label">Remote access</div>
  {#if !status}
    <p class="muted">Not available from this daemon.</p>
  {:else}
    <label class="opt">
      <input type="checkbox" checked={status.enabled} disabled={busy} onchange={(e) => setEnabled((e.currentTarget as HTMLInputElement).checked)} />
      <span>
        <b>Use Muster from your phone or tablet over Tailscale</b>
        <small>
          Sessions keep running on this {computer()}; other devices only show the board and terminals. Only devices you pair can connect, and only through your
          tailnet.
        </small>
      </span>
    </label>

    {#if status.enabled}
      {#if busy}
        <small>Setting up Tailscale…</small>
      {:else if status.error}
        <p class="warn">{status.error}</p>
      {:else if status.url}
        <div class="row">
          <code>{status.url}</code>
          <button class="btn small" onclick={() => copy(status!.url!)}>Copy</button>
        </div>
      {/if}

      {#if status.url && !status.error}
        {#if pairing}
          <div class="pair">
            <div class="qr" aria-label="Pairing QR code">{@html pairing.svg}</div>
            <div class="pair-text">
              <b>Scan with the device's camera</b>
              <small>Or open this link on it. It works once and expires in {minutesLeft} min.</small>
              <div class="row">
                <code class="link">{pairing.url}</code>
                <button class="btn small" onclick={() => copy(pairing!.url)}>Copy</button>
              </div>
              <small>On an iPhone or iPad, use Share → Add to Home Screen afterwards to open Muster like an app.</small>
              <button class="btn small" onclick={() => (pairing = null)}>Cancel</button>
            </div>
          </div>
        {:else}
          <div><button class="btn small" onclick={pair}>Pair a device…</button></div>
        {/if}
      {/if}

      {#if status.devices.length}
        <ul class="devices">
          {#each status.devices as d (d.id)}
            <li>
              <span class="name">{d.name}</span>
              <small>{d.lastSeenAt ? `last seen ${agoLabel(d.lastSeenAt, now)}` : "never connected"}</small>
              <button class="btn small" onclick={() => revoke(d.id, d.name)}>Unpair</button>
            </li>
          {/each}
        </ul>
      {/if}
      <small>The {computer()} must stay awake and connected for devices to reach it.</small>
    {/if}
  {/if}
</section>

<style>
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
  .opt span,
  .pair-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .opt b,
  .pair-text b {
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
    user-select: text;
    -webkit-user-select: text;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  code {
    font-family: var(--mono);
    font-size: 12px;
    user-select: text;
    -webkit-user-select: text;
    word-break: break-all;
    min-width: 0;
  }
  .link {
    font-size: 11px;
  }
  .pair {
    display: flex;
    gap: 14px;
    align-items: flex-start;
  }
  .pair-text {
    gap: 6px;
    min-width: 0;
    align-items: flex-start;
  }
  .qr {
    flex: none;
    width: 168px;
    height: 168px;
    background: #fff;
    border-radius: 6px;
    padding: 4px;
  }
  .qr :global(svg) {
    width: 100%;
    height: 100%;
    display: block;
  }
  .devices {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .devices li {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .name {
    font-weight: 500;
  }
  .devices small {
    flex: 1;
  }
</style>
