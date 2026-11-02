import { useEffect, type ReactNode } from "react";

export function Modal({
  onClose,
  label,
  className = "",
  children,
}: {
  onClose: () => void;
  label: string;
  className?: string;
  children: ReactNode;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-40 bg-black/20" onMouseDown={onClose}>
      <div
        role="dialog"
        aria-modal="true"
        aria-label={label}
        onMouseDown={(e) => e.stopPropagation()}
        className={`absolute left-1/2 -translate-x-1/2 overflow-hidden rounded-xl bg-raised shadow-pop max-[700px]:max-w-[calc(100%-16px)] ${className}`}
      >
        {children}
      </div>
    </div>
  );
}
