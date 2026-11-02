import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { useMail } from "../store";
import { Kbd } from "./primitives";

const VISIBLE_MS = 6000;

/** Bottom notice with Undo after archive/delete. ⌘Z works too, even after it fades. */
export function Toast() {
  const { t } = useTranslation();
  const toast = useMail((s) => s.toast);
  const undo = useMail((s) => s.undo);
  const dismiss = useMail((s) => s.dismissToast);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(dismiss, VISIBLE_MS);
    return () => window.clearTimeout(timer);
  }, [toast, dismiss]);

  if (!toast) return null;
  return (
    <div
      role="status"
      className="fixed bottom-5 left-1/2 z-50 flex -translate-x-1/2 items-center gap-3 rounded-xl bg-raised py-2 pr-2 pl-4 text-[13px] shadow-pop"
    >
      <span>{t(toast.message)}</span>
      {toast.undoable && (
        <button
          type="button"
          onClick={undo}
          className="flex items-center gap-1.5 rounded-md px-2 py-1 font-medium text-accent-text hover:bg-hover"
        >
          {t("toast.undo")}
          <Kbd>⌘Z</Kbd>
        </button>
      )}
      <button
        type="button"
        onClick={dismiss}
        aria-label={t("toast.dismiss")}
        className="flex size-6 items-center justify-center rounded text-text-3 hover:bg-hover hover:text-text"
      >
        <X size={14} />
      </button>
    </div>
  );
}
