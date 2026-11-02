import { Menu, type MenuOptions } from "@tauri-apps/api/menu";
import { create } from "zustand";
import { IS_ANDROID } from "./core";

type MenuItems = NonNullable<MenuOptions["items"]>;

/** The parts of a menu item an in-app sheet can show. */
export interface SheetItem {
  text?: string;
  item?: unknown;
  enabled?: boolean;
  checked?: boolean;
  action?: (id: string) => void;
  items?: SheetItem[];
}

/** The menu open as an in-app sheet (Android has no native context menus). */
export const useMenuSheet = create<{ items: SheetItem[] | null }>(() => ({ items: null }));

/** Shows a context menu: native where there is one, else an in-app sheet. */
export async function popupMenu(items: MenuItems) {
  if (IS_ANDROID) {
    useMenuSheet.setState({ items: items as SheetItem[] });
    return;
  }
  const menu = await Menu.new({ items });
  await menu.popup();
}
