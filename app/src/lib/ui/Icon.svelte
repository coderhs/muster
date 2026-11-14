<script lang="ts" module>
  // Hand-drawn 16px stroke icons.
  const PATHS: Record<string, string> = {
    plus: "M8 3v10M3 8h10",
    close: "M4 4l8 8M12 4l-8 8",
    more: "M3.5 8h.01M8 8h.01M12.5 8h.01",
    folder: "M2 4.5A1.5 1.5 0 0 1 3.5 3h3l1.5 1.5h4.5A1.5 1.5 0 0 1 14 6v5.5a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 11.5z",
    branch: "M5 3v7M5 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4zM11 6a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM11 6c0 3-6 2-6 4",
    play: "M5 3.5v9l7-4.5z",
    stop: "M4.5 4.5h7v7h-7z",
    trash: "M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.5 8.5h6l.5-8.5",
    calendar: "M3 4.5h10v8.5H3zM3 7h10M5.5 3v2.5M10.5 3v2.5",
    flag: "M4 14V3M4 3h7l-1.5 3L11 9H4",
    chevronLeft: "M10 3.5L5.5 8l4.5 4.5",
    chevronRight: "M6 3.5l4.5 4.5L6 12.5",
    terminal: "M3.5 5l3 3-3 3M8 11.5h4.5",
    pencil: "M10.5 3l2.5 2.5L6 12.5H3.5V10z",
    dot: "M8 8h.01",
    sidebar: "M2.5 3.5h11v9h-11zM6 3.5v9",
    warning: "M8 2.5l6 11H2zM8 6.5v3M8 11.5h.01",
    check: "M3.5 8.5l3 3 6-7",
    link: "M7 9a2.5 2.5 0 0 0 3.5 0l2-2A2.5 2.5 0 0 0 9 3.5l-.7.7M9 7a2.5 2.5 0 0 0-3.5 0l-2 2A2.5 2.5 0 0 0 7 12.5l.7-.7",
    mail: "M2.5 4.5h11v7h-11zM2.5 5l5.5 4 5.5-4",
    gear: "M8 5.8a2.2 2.2 0 1 0 0 4.4 2.2 2.2 0 0 0 0-4.4zM8 1.8v1.6M8 12.6v1.6M3.6 3.6l1.1 1.1M11.3 11.3l1.1 1.1M1.8 8h1.6M12.6 8h1.6M3.6 12.4l1.1-1.1M11.3 4.7l1.1-1.1",
    restore: "M3 8a5 5 0 1 0 1.5-3.5M3 2.5v2.5h2.5",
    handoff: "M2.5 8h8M8 5l3 3-3 3M13.5 3v10",
    cpu: "M5 5h6v6H5zM6.5 2.5v2M9.5 2.5v2M6.5 11.5v2M9.5 11.5v2M2.5 6.5h2M2.5 9.5h2M11.5 6.5h2M11.5 9.5h2",
  };
  export type IconName = keyof typeof PATHS;
</script>

<script lang="ts">
  let { name, size = 16, stroke = 1.5 }: { name: IconName; size?: number; stroke?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill="none"
  stroke="currentColor"
  stroke-width={name === "more" || name === "dot" ? 2.6 : stroke}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"><path d={PATHS[name]} /></svg>
