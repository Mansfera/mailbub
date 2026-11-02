import { useEffect, useRef, useState } from "react";

/** Distance (px) a row must travel for the swipe action to fire on release. */
export const SWIPE_THRESHOLD = 96;
const MAX = 140;
/** Trackpad wheel events stop arriving this long after the gesture (incl. momentum) ends. */
const END_DELAY = 140;
const COOLDOWN = 500;

// Shared across rows: after one row is swiped away, leftover momentum must not hit the
// row that slides up under the pointer.
let cooldownUntil = 0;

/**
 * Two-finger horizontal trackpad swipe on an element, read from wheel deltaX (WebKit has no
 * gesture events for this). `offset` < 0 means the content moved left.
 */
export function useSwipe<T extends HTMLElement>(onLeft: () => void, onRight: () => void) {
  const ref = useRef<T>(null);
  const [offset, setOffset] = useState(0);
  const [dragging, setDragging] = useState(false);
  const handlers = useRef({ onLeft, onRight });

  useEffect(() => {
    handlers.current = { onLeft, onRight };
  }, [onLeft, onRight]);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let acc = 0;
    let horizontal: boolean | null = null;
    let endTimer = 0;
    let flyTimer = 0;

    const finish = () => {
      horizontal = null;
      const travelled = acc;
      acc = 0;
      setDragging(false);
      if (travelled <= -SWIPE_THRESHOLD) {
        cooldownUntil = Date.now() + COOLDOWN;
        setOffset(-el.offsetWidth);
        flyTimer = window.setTimeout(() => {
          handlers.current.onLeft();
          setOffset(0);
        }, 180);
      } else {
        if (travelled >= SWIPE_THRESHOLD) {
          cooldownUntil = Date.now() + COOLDOWN;
          handlers.current.onRight();
        }
        setOffset(0);
      }
    };

    const onWheel = (e: WheelEvent) => {
      if (Date.now() < cooldownUntil) return;
      if (horizontal === null) {
        if (Math.abs(e.deltaX) < 2 && Math.abs(e.deltaY) < 2) return;
        horizontal = Math.abs(e.deltaX) > Math.abs(e.deltaY) * 1.5;
      }
      window.clearTimeout(endTimer);
      if (!horizontal) {
        // A vertical scroll: ignore until it ends.
        endTimer = window.setTimeout(() => (horizontal = null), END_DELAY);
        return;
      }
      acc = Math.max(-MAX, Math.min(MAX, acc - e.deltaX));
      setDragging(true);
      setOffset(acc);
      endTimer = window.setTimeout(finish, END_DELAY);
    };

    el.addEventListener("wheel", onWheel, { passive: true });
    return () => {
      el.removeEventListener("wheel", onWheel);
      window.clearTimeout(endTimer);
      window.clearTimeout(flyTimer);
    };
  }, []);

  return { ref, offset, dragging };
}
