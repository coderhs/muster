// One shared clock for every "for 3m" timer. Ticks once a second while the page is
// visible and stops entirely when it is hidden, so idle windows cost nothing.

class Clock {
  now = $state(Date.now());
  private timer: ReturnType<typeof setInterval> | null = null;

  start() {
    const tick = () => (this.now = Date.now());
    const run = () => {
      if (this.timer) clearInterval(this.timer);
      this.timer = null;
      if (document.visibilityState === "visible") {
        tick();
        this.timer = setInterval(tick, 1000);
      }
    };
    document.addEventListener("visibilitychange", run);
    run();
  }
}

export const clock = new Clock();
