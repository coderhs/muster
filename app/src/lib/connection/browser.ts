import type { Platform } from "./index";

const TOKEN_KEY = "muster.token";
const URL_KEY = "muster.url";

/** Defaults that match `npm run mock` so `npm run dev` connects with no setup. */
const DEV_URL = "ws://127.0.0.1:7317/ws";
const DEV_TOKEN = "dev";

function store(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    /* storage disabled */
  }
}

function load(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

/** A name for this device in Muster's list of paired devices. */
export function deviceName(ua: string, touch: boolean): string {
  if (/iPad/.test(ua) || (/Macintosh/.test(ua) && touch)) return "iPad";
  if (/iPhone/.test(ua)) return "iPhone";
  if (/Android/.test(ua)) return /Mobile/.test(ua) ? "Android phone" : "Android tablet";
  if (/Macintosh|Mac OS X/.test(ua)) return "Mac";
  if (/Windows/.test(ua)) return "Windows PC";
  if (/CrOS/.test(ua)) return "Chromebook";
  if (/Linux/.test(ua)) return "Linux computer";
  return "Browser";
}

const NOT_PAIRED =
  "This device is not paired. On your Mac, open Muster → Settings → Remote access → Pair a device, then scan the code or open the link here.";

/** Trades a one-time pairing code for this device's own token. */
async function pairDevice(code: string): Promise<string> {
  const name = deviceName(navigator.userAgent, navigator.maxTouchPoints > 1);
  const r = await fetch("/pair", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ code, name }),
  });
  if (!r.ok) {
    const text = (await r.text().catch(() => "")) || `pairing failed (${r.status})`;
    throw new Error(`${text}. Make a new pairing link on your Mac.`);
  }
  const { token } = (await r.json()) as { token: string };
  return token;
}

/**
 * Remote access: a page served by the daemon through Tailscale pairs with `#pair=<code>`
 * and then keeps its device token; it never asks for the master token.
 *
 * URL and token come from, in order: `?daemon=` / `#token=` on the page URL (the
 * token is then moved out of the address bar), saved values, the page's own host
 * (when the daemon serves the page), and finally a prompt for the token.
 */
export function browserPlatform(): Platform {
  const here = new URL(window.location.href);
  const hash = new URLSearchParams(here.hash.slice(1));
  const qToken = hash.get("token") ?? here.searchParams.get("token");
  const qUrl = here.searchParams.get("daemon");
  if (qToken) store(TOKEN_KEY, qToken);
  if (qUrl) store(URL_KEY, qUrl);
  if (qToken) {
    here.hash = "";
    here.searchParams.delete("token");
    history.replaceState(null, "", here.toString());
  }

  const openOnLoad = hash.get("open");
  let pairCode = hash.get("pair");
  let pairError: string | null = null;
  if (pairCode) {
    here.hash = "";
    history.replaceState(null, "", here.toString());
  }
  const remote = !import.meta.env.DEV && location.protocol === "https:" && !["localhost", "127.0.0.1"].includes(location.hostname);
  return {
    kind: "browser",
    async openUrl(url) {
      window.open(url, "_blank", "noopener,noreferrer");
    },
    onOpenCard(cb) {
      if (openOnLoad) cb(openOnLoad);
    },
    async resolve(lastError) {
      let url = load(URL_KEY);
      if (!url) {
        url = import.meta.env.DEV
          ? DEV_URL
          : `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`;
      }
      if (remote) {
        if (pairCode) {
          const code = pairCode;
          pairCode = null;
          try {
            store(TOKEN_KEY, await pairDevice(code));
            pairError = null;
          } catch (e) {
            pairError = e instanceof Error ? e.message : String(e);
          }
        } else if (lastError?.code === "unauthorized") {
          store(TOKEN_KEY, null);
        }
        const token = load(TOKEN_KEY);
        if (!token) throw new Error(pairError ?? NOT_PAIRED);
        return { url, token };
      }
      let token = load(TOKEN_KEY) ?? (import.meta.env.DEV ? DEV_TOKEN : null);
      if (lastError?.code === "unauthorized" || !token) {
        const entered = window.prompt(
          lastError?.code === "unauthorized" ? "The daemon rejected the token. Enter the Muster token:" : "Enter the Muster token:",
          "",
        );
        if (entered === null) throw new Error("no token entered");
        token = entered.trim();
        store(TOKEN_KEY, token);
      }
      return { url, token };
    },
  };
}
