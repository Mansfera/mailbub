import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, Paperclip } from "lucide-react";
import { core, CoreError, IN_TAURI } from "../lib/core";
import { formatSize } from "../lib/format";
import { useMail } from "../store";
import type { Attachment } from "../types";

/** File chip; the first click downloads it from the server, then it opens in its app. */
export function AttachmentChip({
  messageId,
  attachment: a,
  onAccent,
  disabled = false,
}: {
  messageId: string;
  attachment: Attachment;
  onAccent: boolean;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const notify = useMail((s) => s.notify);
  const [busy, setBusy] = useState(false);

  const open = async () => {
    if (!IN_TAURI || busy) return;
    setBusy(true);
    try {
      await core.openAttachment(messageId, a.id);
    } catch (e) {
      notify(
        `${t("attachment.failed")}${e instanceof CoreError && e.message ? `: ${e.message}` : ""}`,
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <button
      type="button"
      title={disabled ? a.name : `${a.name} · ${t("attachment.open")}`}
      onClick={(e) => {
        e.stopPropagation();
        void open();
      }}
      aria-busy={busy}
      disabled={disabled}
      className={`flex max-w-full items-center gap-1.5 rounded-lg px-2 py-1 text-[12px] ${
        onAccent ? "bg-white/15 enabled:hover:bg-white/25" : "bg-raised enabled:hover:bg-hover"
      }`}
    >
      {busy ? (
        <Loader2 size={13} aria-hidden className="shrink-0 animate-spin" />
      ) : (
        <Paperclip size={13} aria-hidden className="shrink-0" />
      )}
      <span className="truncate">{a.name}</span>
      <span className={onAccent ? "text-white/70" : "text-text-2"}>{formatSize(a.size)}</span>
    </button>
  );
}
