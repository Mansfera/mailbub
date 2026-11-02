import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronsUpDown, Loader2 } from "lucide-react";
import { useMail } from "../store";
import { accountAddresses } from "../lib/chat";
import { IN_TAURI, IS_ANDROID } from "../lib/core";
import { toUploads, useDraftFiles, useFileDrop } from "../lib/draftFiles";
import { AccountDot } from "./primitives";
import { AttachButton, DraftFileList, DropOverlay } from "./DraftFiles";
import { Modal } from "./Modal";

export function ComposeDialog() {
  const { t } = useTranslation();
  const accounts = useMail((s) => s.accounts);
  const accountFilter = useMail((s) => s.accountFilter);
  const setComposeOpen = useMail((s) => s.setComposeOpen);
  const sendMessage = useMail((s) => s.sendMessage);
  // One menu for every address of every account, as `accountId|address`.
  const [sender, setSender] = useState(() => {
    const id = accountFilter === "all" ? accounts[0].id : accountFilter;
    return `${id}|${accounts.find((a) => a.id === id)?.defaultFrom ?? ""}`;
  });
  const [accountId, from] = sender.split("|");
  const [to, setTo] = useState("");
  const [subject, setSubject] = useState("");
  const [body, setBody] = useState("");
  const [sending, setSending] = useState(false);
  const files = useDraftFiles();
  const form = useRef<HTMLFormElement>(null);
  const dragging = useFileDrop(form, files.add);
  const close = () => setComposeOpen(false);
  const account = accounts.find((a) => a.id === accountId);
  const canSend =
    to.trim().length > 0 &&
    (body.trim().length > 0 || files.files.length > 0) &&
    !sending &&
    IN_TAURI;

  const send = async () => {
    if (!canSend) return;
    setSending(true);
    // Success closes the dialog; on failure the draft stays.
    const attachments = await toUploads(files.files);
    const sent = await sendMessage({ accountId, from, to, subject, body, attachments });
    if (!sent) setSending(false);
  };

  const field = "h-8 flex-1 bg-transparent text-[13px] outline-none placeholder:text-text-3";
  const row = "flex items-center gap-3 border-b border-line px-4";
  const labelCls = "w-14 shrink-0 text-[12px] text-text-2";

  return (
    <Modal onClose={close} label={t("compose.title")} className="top-[10vh] w-[560px]">
      <form
        ref={form}
        className="relative"
        // Addresses are checked by the core, which also accepts ";" between them.
        noValidate
        onSubmit={(e) => {
          e.preventDefault();
          void send();
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void send();
          }
        }}
      >
        <h2 className="px-4 pt-3 pb-2 text-[15px] font-semibold">{t("compose.title")}</h2>
        <div className={`${row} border-t`}>
          <label htmlFor="compose-from" className={labelCls}>
            {t("compose.from")}
          </label>
          {account && <AccountDot color={account.color} />}
          {/* appearance-none drops WebKit's popup-button bezel so it reads like the other fields. */}
          <div className="relative flex flex-1 items-center">
            <select
              id="compose-from"
              value={sender}
              onChange={(e) => setSender(e.target.value)}
              className={`${field} cursor-default appearance-none pr-6`}
            >
              {accounts.flatMap((a) =>
                accountAddresses(a).map((address) => (
                  <option key={`${a.id}|${address}`} value={`${a.id}|${address}`}>
                    {address}
                  </option>
                )),
              )}
            </select>
            <ChevronsUpDown
              size={14}
              className="pointer-events-none absolute right-0 text-text-3"
              aria-hidden
            />
          </div>
        </div>
        <div className={row}>
          <label htmlFor="compose-to" className={labelCls}>
            {t("compose.to")}
          </label>
          <input
            id="compose-to"
            autoFocus
            type="email"
            multiple
            value={to}
            onChange={(e) => setTo(e.target.value)}
            placeholder={t("compose.toPlaceholder")}
            className={`selectable ${field}`}
          />
        </div>
        <div className={row}>
          <label htmlFor="compose-subject" className={labelCls}>
            {t("compose.subject")}
          </label>
          <input
            id="compose-subject"
            value={subject}
            onChange={(e) => setSubject(e.target.value)}
            className={`selectable ${field}`}
          />
        </div>
        <textarea
          aria-label={t("compose.body")}
          value={body}
          onChange={(e) => setBody(e.target.value)}
          placeholder={t("compose.body")}
          className="selectable block h-48 w-full resize-none bg-transparent px-4 py-3 text-[14px] leading-6 outline-none placeholder:text-text-3"
        />
        <DraftFileList
          files={files.files}
          onRemove={files.remove}
          className="max-h-24 overflow-y-auto px-4 pb-2.5"
        />
        <div className="flex items-center gap-2 border-t border-line py-2.5 pr-4 pl-2.5">
          <AttachButton onFiles={files.add} />
          <span className="flex-1" />
          <button
            type="button"
            onClick={close}
            className="h-7 rounded-md px-3 text-[13px] text-text-2 hover:bg-hover"
          >
            {t("compose.cancel")}
          </button>
          <button
            type="submit"
            disabled={!canSend}
            className="flex h-7 items-center gap-2 rounded-md bg-accent px-3 text-[13px] font-medium text-on-accent hover:bg-accent-hover disabled:opacity-40"
          >
            {sending && <Loader2 size={13} className="animate-spin" aria-hidden />}
            {t(sending ? "compose.sending" : "compose.send")}
            {!sending && !IS_ANDROID && <span className="text-[11px] text-white/75">⌘↵</span>}
          </button>
        </div>
        <DropOverlay show={dragging} />
      </form>
    </Modal>
  );
}
