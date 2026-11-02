import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { Paperclip, X } from "lucide-react";
import { formatSize } from "../lib/format";
import { IconButton } from "./primitives";

/** Paperclip that opens the file picker. */
export function AttachButton({ onFiles }: { onFiles: (files: File[]) => void }) {
  const { t } = useTranslation();
  const input = useRef<HTMLInputElement>(null);
  return (
    <>
      <IconButton label={t("attach.add")} onClick={() => input.current?.click()}>
        <Paperclip size={15} aria-hidden />
      </IconButton>
      <input
        ref={input}
        type="file"
        multiple
        hidden
        tabIndex={-1}
        onChange={(e) => {
          onFiles(Array.from(e.target.files ?? []));
          // Picking the same file again after removing it should still fire onChange.
          e.target.value = "";
        }}
      />
    </>
  );
}

/** Files on a draft, each with a remove button. */
export function DraftFileList({
  files,
  onRemove,
  className = "",
}: {
  files: File[];
  onRemove: (index: number) => void;
  className?: string;
}) {
  const { t } = useTranslation();
  if (!files.length) return null;
  return (
    <ul aria-label={t("attach.list")} className={`flex flex-wrap gap-1.5 ${className}`}>
      {files.map((f, i) => (
        <li
          // Same file may be attached twice; the index keeps keys unique.
          key={`${i}-${f.name}-${f.size}`}
          className="flex max-w-[260px] items-center gap-1.5 rounded-lg bg-surface py-1 pr-1 pl-2 text-[12px]"
        >
          <Paperclip size={13} aria-hidden className="shrink-0 text-text-2" />
          <span className="truncate" title={f.name}>
            {f.name}
          </span>
          <span className="shrink-0 text-text-2">{formatSize(f.size)}</span>
          <button
            type="button"
            onClick={() => onRemove(i)}
            aria-label={t("attach.remove", { name: f.name })}
            title={t("attach.remove", { name: f.name })}
            className="flex size-5 shrink-0 items-center justify-center rounded text-text-2 hover:bg-hover hover:text-text"
          >
            <X size={12} aria-hidden />
          </button>
        </li>
      ))}
    </ul>
  );
}

/** Covers a drop target while files are dragged over it (also over email frames). */
export function DropOverlay({ show }: { show: boolean }) {
  const { t } = useTranslation();
  if (!show) return null;
  return (
    // Takes the drag events itself, so they don't go into an email iframe underneath.
    <div className="absolute inset-2 z-30 flex items-center justify-center rounded-xl border-2 border-dashed border-brand bg-bg/85">
      <div className="flex items-center gap-2 text-[14px] font-medium text-accent-text">
        <Paperclip size={16} aria-hidden />
        {t("attach.drop")}
      </div>
    </div>
  );
}
