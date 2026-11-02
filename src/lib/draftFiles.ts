import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import i18n from "../i18n";
import { useMail } from "../store";
import { formatSize } from "./format";

/** Mirrors `send::MAX_ATTACHMENTS_BYTES`: the total for one message. */
export const MAX_ATTACHMENTS_BYTES = 25 * 1024 * 1024;

/** A file as the core takes it: bytes in base64. */
export interface FileUpload {
  name: string;
  mime: string;
  data: string;
}

function readBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    // A data: URL is "data:<type>;base64,<data>"; only the data goes to the core.
    reader.onload = () => resolve(String(reader.result).split(",", 2)[1] ?? "");
    reader.onerror = () => reject(reader.error ?? new Error(file.name));
    reader.readAsDataURL(file);
  });
}

export async function toUploads(files: File[]): Promise<FileUpload[]> {
  return Promise.all(
    files.map(async (f) => ({ name: f.name, mime: f.type, data: await readBase64(f) })),
  );
}

/** Files attached to a draft; refuses ones that would take it over the size limit. */
export function useDraftFiles() {
  const [files, setFiles] = useState<File[]>([]);
  const add = useCallback((added: File[]) => {
    setFiles((prev) => {
      const next = [...prev];
      let total = prev.reduce((n, f) => n + f.size, 0);
      for (const f of added) {
        // Folders come through a drop as empty files with no type.
        if (f.size === 0 && !f.type) continue;
        if (total + f.size > MAX_ATTACHMENTS_BYTES) {
          useMail
            .getState()
            .notify(
              i18n.t("attach.tooBig", { name: f.name, max: formatSize(MAX_ATTACHMENTS_BYTES) }),
            );
          continue;
        }
        total += f.size;
        next.push(f);
      }
      return next;
    });
  }, []);
  const remove = useCallback((i: number) => setFiles((p) => p.filter((_, j) => j !== i)), []);
  const clear = useCallback(() => setFiles([]), []);
  return { files, add, remove, clear };
}

const hasFiles = (e: DragEvent) => e.dataTransfer?.types.includes("Files") ?? false;

/** Makes an element a drop target for files; true while files are dragged over it. */
export function useFileDrop(
  target: RefObject<HTMLElement | null>,
  onFiles: (files: File[]) => void,
): boolean {
  const [over, setOver] = useState(false);
  const onFilesRef = useRef(onFiles);
  useEffect(() => {
    onFilesRef.current = onFiles;
  }, [onFiles]);

  useEffect(() => {
    const el = target.current;
    if (!el) return;
    // dragenter/dragleave also fire on every child crossed; count them to know when it left.
    let depth = 0;
    const enter = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      depth++;
      setOver(true);
    };
    const move = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    };
    const leave = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      depth = Math.max(0, depth - 1);
      if (depth === 0) setOver(false);
    };
    const drop = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      depth = 0;
      setOver(false);
      const files = Array.from(e.dataTransfer?.files ?? []);
      if (files.length) onFilesRef.current(files);
    };
    el.addEventListener("dragenter", enter);
    el.addEventListener("dragover", move);
    el.addEventListener("dragleave", leave);
    el.addEventListener("drop", drop);
    return () => {
      el.removeEventListener("dragenter", enter);
      el.removeEventListener("dragover", move);
      el.removeEventListener("dragleave", leave);
      el.removeEventListener("drop", drop);
    };
  }, [target]);

  return over;
}

/** Files dropped outside a drop target would open in the webview in place of the app. */
export function blockStrayDrops() {
  for (const type of ["dragover", "drop"] as const) {
    window.addEventListener(type, (e) => {
      if (e.defaultPrevented) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "none";
    });
  }
}
