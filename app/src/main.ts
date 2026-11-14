import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";
import { detectPlatform } from "./lib/connection";
import { AppState } from "./lib/state/app.svelte";
import { clock } from "./lib/state/clock.svelte";

// The engine's own context menu (Reload, Inspect Element...) is not part of the app; keep it
// only where it is useful: text fields, the terminal, and selected text.
document.addEventListener("contextmenu", (e) => {
  const t = e.target as HTMLElement;
  if (t.closest("input, textarea, [contenteditable], .xterm") || String(window.getSelection() ?? "")) return;
  e.preventDefault();
});

const platform = await detectPlatform();
const app = new AppState(platform);
clock.start();
mount(App, { target: document.getElementById("app")!, props: { app } });
app.start();
platform.onOpenCard?.((id) => void app.openCard(id));
