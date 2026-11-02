import { useEffect, useSyncExternalStore } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import i18n from "../i18n";
import { usePrefs } from "../store";
import { core, IN_TAURI, IS_ANDROID } from "./core";

const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

function subscribeDark(cb: () => void) {
  darkQuery.addEventListener("change", cb);
  return () => darkQuery.removeEventListener("change", cb);
}

function useSystemDark(): boolean {
  return useSyncExternalStore(subscribeDark, () => darkQuery.matches);
}

/** Applies the theme preference to the page and the native window. */
export function useTheme() {
  const theme = usePrefs((s) => s.theme);
  const systemDark = useSystemDark();
  const resolved = theme === "system" ? (systemDark ? "dark" : "light") : theme;

  useEffect(() => {
    document.documentElement.dataset.theme = resolved;
    if (IS_ANDROID) void core.setSystemBars(resolved === "dark").catch(() => undefined);
  }, [resolved]);

  // The native window (vibrancy material, traffic lights) follows our choice too.
  useEffect(() => {
    if (IN_TAURI && !IS_ANDROID)
      void getCurrentWindow().setTheme(theme === "system" ? null : theme);
  }, [theme]);
}

export function useLanguage() {
  const language = usePrefs((s) => s.language);
  useEffect(() => {
    void i18n.changeLanguage(language);
    document.documentElement.lang = language;
  }, [language]);
}

/** Preferences live in localStorage; pick up changes made in the other window. */
export function usePrefsFromOtherWindows() {
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key === "mailbub.prefs") void usePrefs.persist.rehydrate();
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);
}
