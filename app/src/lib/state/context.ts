import { getContext, setContext } from "svelte";
import type { AppState } from "./app.svelte";

const KEY = Symbol("muster-app");

export const provideApp = (app: AppState) => setContext(KEY, app);
export const useApp = () => getContext<AppState>(KEY);
