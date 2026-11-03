/*
 * Dev-only stand-in for the Rust core, so `bun run dev` can show the real UI with sample
 * chats in a plain browser. Opt in with `?mock` in the URL; production builds drop it.
 */
// Everything sits inside one function so production builds drop the sample data with it.
if (import.meta.env.DEV && new URLSearchParams(location.search).has("mock")) install();

function install() {
  type Args = Record<string, unknown> | undefined;

  const now = Math.floor(Date.now() / 1000);
  const min = 60;
  const hour = 60 * min;
  const day = 24 * hour;

  const person = (name: string, address: string) => ({ name, address });
  const me = person("Volo Nehoda", "volo@etherlynx.xyz");

  const accounts = [
    account("work", "Work", "volo@etherlynx.xyz", "teal", "imap"),
    account("gmail", "Personal", "nehoda.volo@gmail.com", "red", "gmail"),
    account("uni", "University", "v.nehoda@tuwien.ac.at", "gold", "microsoft"),
  ];

  function account(id: string, label: string, email: string, color: string, provider: string) {
    const server = { host: "imap.example.com", port: 993, security: "tls" };
    return {
      id,
      provider,
      label,
      email,
      username: email,
      displayName: me.name,
      aliases: [],
      defaultFrom: email,
      color,
      imap: server,
      smtp: { ...server, port: 465 },
      auth: "password",
      createdAt: now - 90 * day,
    };
  }

  interface Seed {
    id: string;
    accountId: string;
    with: { name: string; address: string }[];
    subject: string;
    /** [outgoing, seconds ago, body] oldest first. */
    thread: [boolean, number, string][];
    unread?: number;
    pinned?: boolean;
    muted?: boolean;
    automated?: boolean;
    manualStatus?: string;
    attachments?: boolean;
  }

  const seeds: Seed[] = [
    {
      id: "c1",
      accountId: "work",
      with: [person("Taylor Thigpen", "taylor@tayloredglobal.com")],
      subject: "Speaker kit — final pass",
      pinned: true,
      unread: 2,
      attachments: true,
      thread: [
        [
          true,
          2 * day,
          "Hi Taylor, the new speaker kit is up on the review link. Could you check the bio section and the photo crops?",
        ],
        [
          false,
          26 * hour,
          "Thanks! Looks great overall. Two notes: the bio should open with the Fortune 500 work, and the second photo feels too dark.",
        ],
        [true, 25 * hour, "Got it, swapping the order now. I'll brighten the photo too."],
        [
          false,
          40 * min,
          "Perfect. One more thing: can we add the podcast logos to the last page? Attaching them here.",
        ],
        [false, 38 * min, "And the deadline moved — the event team needs it by Thursday."],
      ],
    },
    {
      id: "c2",
      accountId: "gmail",
      with: [person("Olena Kovalenko", "olena.k@kidskonnect.org")],
      subject: "Ep.02 guest confirmed",
      unread: 1,
      thread: [
        [false, 3 * hour, "Nuno said yes for the 14th! He asked for the questions a week before."],
        [true, 2 * hour, "Amazing news. I'll draft the brief tonight and send it to you first."],
        [false, 12 * min, "Great — also, can we record at 18:00 Vienna time instead of 17:00?"],
      ],
    },
    {
      id: "c3",
      accountId: "work",
      with: [
        person("Ihor Melnyk", "ihor@gaplight.site"),
        person("Anna Shevchuk", "anna@gaplight.site"),
      ],
      subject: "Algorithm benchmark",
      thread: [
        [
          true,
          5 * hour,
          "Sharing the AUC benchmark: the model is at 0.720 and loses to the baseline below 50% confidence. Script is in the repo.",
        ],
        [
          false,
          4 * hour,
          "Ouch. I'll look at the tag accuracy formula, those 182% values are definitely a bug.",
        ],
        [false, 3 * hour, "I can pair on it tomorrow morning if that helps."],
        [true, 2.5 * hour, "Yes please, 10:00 works for me."],
      ],
    },
    {
      id: "c4",
      accountId: "uni",
      with: [person("Prof. Maria Huber", "maria.huber@tuwien.ac.at")],
      subject: "Seminar presentation slot",
      manualStatus: "needs_reply",
      thread: [
        [
          false,
          1 * day + 3 * hour,
          "Dear Mr. Nehoda, please choose a slot for your seminar presentation: 12 or 19 November. Kind regards, M. Huber",
        ],
      ],
    },
    {
      id: "c5",
      accountId: "work",
      with: [person("Martina Rossi", "martina@theresidency.nyc")],
      subject: "October update",
      thread: [
        [
          true,
          2 * day + 4 * hour,
          "Hi Martina, quick monthly update:\n\n• Gaplight cohort is live with 3 schools\n• Etherlynx shipped two client sites\n• Applying to a16z Academy this week",
        ],
      ],
    },
    {
      id: "c6",
      accountId: "gmail",
      with: [person("Linear", "notifications@linear.app")],
      subject: "Weekly digest",
      automated: true,
      muted: true,
      thread: [
        [
          false,
          3 * day,
          "You have 4 issues due this week in UKNOW. Ep.02 rundown, Merch preorder page, Casting deck, Guest brief.",
        ],
      ],
    },
    {
      id: "c7",
      accountId: "work",
      with: [person("Yurii Bobak", "y.bobak@aurea.build")],
      subject: "Grand Family Zakarpattia deck",
      manualStatus: "done",
      thread: [
        [false, 4 * day, "Volodymyr, the deck is approved. Thank you, we'll present it on Monday."],
        [true, 4 * day - hour, "Glad to hear! Good luck with the presentation."],
      ],
    },
    {
      id: "c8",
      accountId: "uni",
      with: [person("Lukas Wagner", "lukas.wagner@student.tuwien.ac.at")],
      subject: "Group project",
      thread: [
        [
          false,
          6 * day,
          "Hey, did you finish your part of the report? We need to submit by Friday.",
        ],
        [true, 6 * day - 2 * hour, "Almost, I'll push it tonight."],
      ],
    },
    {
      id: "c9",
      accountId: "gmail",
      with: [person("Substack", "no-reply@substack.com")],
      subject: "What the markets are saying",
      automated: true,
      thread: [[false, 8 * day, "Making prediction market data easier to reference and share."]],
    },
  ];

  function messages(s: Seed) {
    return s.thread.map(([outgoing, ago, body], i) => ({
      id: `${s.id}-m${i}`,
      from: outgoing ? me : s.with[i % s.with.length],
      outgoing,
      ownAddress: null,
      sentAt: Math.round(now - ago),
      subject: i === 0 ? s.subject : `Re: ${s.subject}`,
      body,
      quoted: undefined,
      html: null,
      attachments:
        s.attachments && i === s.thread.length - 2
          ? [
              { id: "a1", name: "podcast-logos.zip", size: 2_480_000 },
              { id: "a2", name: "speaker-kit-v3.pdf", size: 840_000 },
            ]
          : [],
      read: s.thread.length - i > (s.unread ?? 0),
      unsubscribe: null,
      replyTo: null,
    }));
  }

  const state = new Map(seeds.map((s) => [s.id, { ...s }]));

  const chats = () =>
    [...state.values()].map((s) => ({
      id: s.id,
      accountId: s.accountId,
      participants: s.with,
      last: messages(s).at(-1),
      unread: s.unread ?? 0,
      automated: s.automated ?? false,
      pinned: s.pinned ?? false,
      muted: s.muted ?? false,
      archived: false,
      manualStatus: s.manualStatus ?? null,
    }));

  const handlers: Record<string, (a: Args) => unknown> = {
    list_accounts: () => accounts,
    list_chats: () => chats(),
    chat_messages: (a) => {
      const s = state.get(a?.chatId as string);
      return s ? messages(s) : [];
    },
    list_remote_rules: () => [],
    sync_status: () => ({ running: false, lastRun: now - 2 * min, errors: [] }),
    search_chats: () => [],
    load_older_mail: () => [],
    list_mailboxes: () => [],
    set_chat_read: (a) => {
      const s = state.get(a?.chatId as string);
      if (s) s.unread = a?.read ? 0 : 1;
      return null;
    },
    set_chat_state: (a) => {
      const s = state.get(a?.chatId as string);
      const c = a?.chatState as { pinned: boolean; muted: boolean; manualStatus: string | null };
      if (s) Object.assign(s, { pinned: c.pinned, muted: c.muted, manualStatus: c.manualStatus });
      return null;
    },
    ms_oauth_available: () => false,
    google_oauth_available: () => false,
  };

  let nextCallback = 1;
  const callbacks = new Map<number, (v: unknown) => void>();

  Object.assign(window, {
    __MAILBUB_MOCK__: true,
    __TAURI_INTERNALS__: {
      metadata: {
        currentWindow: { label: "main" },
        currentWebview: { label: "main", windowLabel: "main" },
      },
      transformCallback(cb: (v: unknown) => void) {
        const id = nextCallback++;
        callbacks.set(id, cb);
        return id;
      },
      unregisterCallback: (id: number) => callbacks.delete(id),
      convertFileSrc: (path: string) => path,
      invoke: async (cmd: string, args?: Args) => {
        const handler = handlers[cmd];
        if (handler) return structuredClone(handler(args));
        if (cmd.startsWith("plugin:")) return nextCallback++;
        return null;
      },
    },
    __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => undefined },
  });
}
