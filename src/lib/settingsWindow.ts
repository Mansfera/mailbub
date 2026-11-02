import { emit } from "@tauri-apps/api/event";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { create } from "zustand";
import { IN_TAURI, IS_ANDROID } from "./core";
import i18n from "../i18n";

export type SettingsTab = "general" | "accounts" | "mail" | "privacy" | "storage";

/** Settings shown as a page inside the main window (Android has only one window). */
export const useSettingsPage = create<{ open: boolean; tab?: SettingsTab }>(() => ({
  open: false,
}));

/** The tab Settings opens on: asked for by the page state or by the window's URL. */
export function initialSettingsTab(): SettingsTab {
  const fromUrl = window.location.hash.split("/")[1] as SettingsTab | undefined;
  return useSettingsPage.getState().tab ?? fromUrl ?? "general";
}

/** Opens Settings in its own window (on `tab`), or brings the existing one to the front. */
export async function openSettingsWindow(tab?: SettingsTab) {
  if (IS_ANDROID) {
    useSettingsPage.setState({ open: true, tab });
    return;
  }
  const url = tab ? `/#settings/${tab}` : "/#settings";
  if (!IN_TAURI) {
    window.open(url, "_blank");
    return;
  }
  const existing = await WebviewWindow.getByLabel("settings");
  if (existing) {
    await existing.setFocus();
    if (tab) await emit("settings-tab", tab);
    return;
  }
  new WebviewWindow("settings", {
    url,
    title: i18n.t("settings.title"),
    width: 680,
    height: 560,
    minWidth: 560,
    minHeight: 420,
    resizable: true,
    maximizable: false,
    minimizable: false,
    center: true,
    titleBarStyle: "overlay",
    hiddenTitle: true,
  });
}
