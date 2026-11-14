<script lang="ts">
  import type { ProcStats } from "../protocol/types";
  import { bytes, cpu } from "./format";
  let { s }: { s: ProcStats } = $props();
</script>

<span class="st" title="{bytes(s.memBytes)} memory, {cpu(s.cpuPercent)} of one core, {s.processes} process{s.processes === 1 ? '' : 'es'}">
  {bytes(s.memBytes)}<span class="sep">·</span>{cpu(s.cpuPercent)}
</span>

<style>
  .st {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 10.5px;
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .sep {
    opacity: 0.6;
  }
</style>
