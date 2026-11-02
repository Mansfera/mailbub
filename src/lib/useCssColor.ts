import { useSyncExternalStore } from "react";

function subscribe(cb: () => void) {
  const obs = new MutationObserver(cb);
  obs.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  return () => obs.disconnect();
}

/** Resolved value of a CSS custom property on :root, updated when the theme changes. */
export function useCssColor(name: string): string {
  return useSyncExternalStore(subscribe, () =>
    getComputedStyle(document.documentElement).getPropertyValue(name).trim(),
  );
}
