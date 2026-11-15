// Stopping idle sessions: the choices offered in Settings and the words on a card.

export const IDLE_CHOICES: { value: string; label: string }[] = [
  { value: "0", label: "Never" },
  { value: "60", label: "1 hour" },
  { value: "240", label: "4 hours" },
  { value: "720", label: "12 hours" },
  { value: "1440", label: "1 day" },
  { value: "4320", label: "3 days" },
  { value: "10080", label: "7 days" },
];

/** The choices, plus the current value when it is not one of them (set by hand). */
export function idleChoices(minutes: number): { value: string; label: string }[] {
  const v = String(minutes);
  return IDLE_CHOICES.some((c) => c.value === v) ? IDLE_CHOICES : [...IDLE_CHOICES, { value: v, label: idleLabel(minutes) }];
}

/** "90 minutes", "4 hours", "3 days". */
export function idleLabel(minutes: number): string {
  if (minutes <= 0) return "Never";
  const unit = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;
  if (minutes % 1440 === 0) return unit(minutes / 1440, "day");
  if (minutes % 60 === 0) return unit(minutes / 60, "hour");
  return unit(minutes, "minute");
}

/** A card whose session Muster stopped for being idle (and that has not started since). */
export function isSleeping(card: { status: string; idleStoppedAt?: number | null }): boolean {
  return card.status === "exited" && card.idleStoppedAt != null;
}
