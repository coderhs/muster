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

/**
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
