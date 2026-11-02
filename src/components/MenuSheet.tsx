import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronLeft, ChevronRight } from "lucide-react";
import { useMenuSheet, type SheetItem } from "../lib/popupMenu";
import { Modal } from "./Modal";

/** A context menu on touch screens; submenus replace the list, with a way back. */
export function MenuSheet({ items }: { items: SheetItem[] }) {
  const { t } = useTranslation();
  const [path, setPath] = useState<SheetItem[]>([]);
  const close = () => useMenuSheet.setState({ items: null });
  const parent = path[path.length - 1];
  const shown = parent?.items ?? items;

  return (
    <Modal
      onClose={close}
      label={parent?.text ?? t("conversation.actions")}
      className="bottom-2 w-[calc(100%-16px)] max-w-[420px]"
    >
      {parent && (
        <button
          type="button"
          onClick={() => setPath(path.slice(0, -1))}
          className="flex min-h-11 w-full items-center gap-1 border-b border-line px-2 text-left text-[14px] font-medium"
        >
          <ChevronLeft size={18} aria-label={t("setup.back")} />
          <span className="truncate">{parent.text}</span>
        </button>
      )}
      <ul className="max-h-[70dvh] overflow-y-auto py-1">
        {shown.map((item, i) =>
          item.item ? (
            <li key={i} role="separator" className="my-1 border-t border-line" />
          ) : (
            <li key={i}>
              <button
                type="button"
                disabled={item.enabled === false}
                onClick={() => {
                  if (item.items) return setPath([...path, item]);
                  close();
                  item.action?.("");
                }}
                className="flex min-h-11 w-full items-center gap-2 px-4 py-2 text-left text-[14px] active:bg-hover disabled:text-text-3"
              >
                <span className="min-w-0 flex-1">{item.text}</span>
                {item.checked && <Check size={16} className="text-accent-text" aria-hidden />}
                {item.items && <ChevronRight size={16} className="text-text-3" aria-hidden />}
              </button>
            </li>
          ),
        )}
      </ul>
    </Modal>
  );
}
