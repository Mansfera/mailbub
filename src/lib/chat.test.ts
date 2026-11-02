import { describe, expect, test } from "bun:test";
import type { Account, Chat, Message } from "../types";
import { effectiveStatus, isNoReplyChat, isUnsubscribeChat, replyFrom } from "./chat";

const message = (outgoing: boolean): Message => ({
  id: "1",
  from: { name: "Shop", address: "noreply@shop.sk" },
  outgoing,
  sentAt: "2026-10-31T10:00:00Z",
  subject: "Order",
  body: "Shipped",
  attachments: [],
  read: true,
});

const chat = (patch: Partial<Chat> = {}): Chat => ({
  id: "a|noreply@shop.sk",
  accountId: "a",
  participants: [{ name: "Shop", address: "noreply@shop.sk" }],
  last: message(false),
  unread: 0,
  automated: false,
  pinned: false,
  muted: false,
  archived: false,
  ...patch,
});

describe("replyFrom", () => {
  const account = {
    email: "me@example.com",
    aliases: ["hi@alias.example"],
    defaultFrom: "me@example.com",
  } as Account;
  const incoming = (id: string, ownAddress?: string): Message => ({
    ...message(false),
    id,
    ownAddress,
  });
  const sent = (id: string, address: string): Message => ({
    ...message(true),
    id,
    from: { name: "Me", address },
  });

  test("answers from the address the mail was sent to", () => {
    expect(replyFrom(account, [incoming("1", "hi@alias.example")])).toBe("hi@alias.example");
    expect(replyFrom(account, [incoming("1", "me@example.com")])).toBe("me@example.com");
  });

  test("keeps the address the chat last used", () => {
    expect(replyFrom(account, [sent("1", "hi@alias.example"), incoming("2")])).toBe(
      "hi@alias.example",
    );
  });

  test("goes by the message being answered, not the newest", () => {
    const first = incoming("1", "hi@alias.example");
    expect(replyFrom(account, [first, incoming("2", "me@example.com")], first)).toBe(
      "hi@alias.example",
    );
  });

  test("falls back to the default, also for addresses no longer on the account", () => {
    const withDefault = { ...account, defaultFrom: "hi@alias.example" };
    expect(replyFrom(withDefault, [])).toBe("hi@alias.example");
    expect(replyFrom(withDefault, [sent("1", "old@example.com")])).toBe("hi@alias.example");
  });
});

describe("effectiveStatus", () => {
  test("whoever wrote last owes the reply", () => {
    expect(effectiveStatus(chat())).toBe("needs_reply");
    expect(effectiveStatus(chat({ last: message(true) }))).toBe("waiting");
  });

  test("automated mail is done", () => {
    expect(effectiveStatus(chat({ automated: true }))).toBe("done");
  });

  test("manual status wins", () => {
    expect(effectiveStatus(chat({ automated: true, manualStatus: "needs_reply" }))).toBe(
      "needs_reply",
    );
    expect(effectiveStatus(chat({ manualStatus: "done" }))).toBe("done");
  });
});

describe("isNoReplyChat", () => {
  const to = (...addresses: string[]) => addresses.map((address) => ({ name: "", address }));

  // The same addresses as `detects_automated_mail_by_sender` in the Rust core.
  test("no-reply style addresses", () => {
    for (const address of [
      "noreply@diia.gov.ua",
      "no-reply@accounts.google.com",
      "No_Reply@bank.sk",
      "donotreply@university.sk",
      "do-not-reply@shop.sk",
      "MAILER-DAEMON@mx.example.com",
      "notifications@github.com",
      "notification+abc@facebookmail.com",
      "newsletter@shop.sk",
      "noreply-orders@shop.sk",
    ]) {
      expect(isNoReplyChat(chat({ participants: to(address) }))).toBe(true);
    }
  });

  test("addresses people read", () => {
    for (const address of [
      "info@studio.sk",
      "support@shop.sk",
      "reply@x.sk",
      "noreplyman@x.sk",
      "nora@x.sk",
    ]) {
      expect(isNoReplyChat(chat({ participants: to(address) }))).toBe(false);
    }
  });

  test("everyone in a group has to be a no-reply address", () => {
    expect(isNoReplyChat(chat({ participants: to("noreply@a.sk", "bounce@b.sk") }))).toBe(true);
    expect(isNoReplyChat(chat({ participants: to("noreply@a.sk", "anna@b.sk") }))).toBe(false);
  });

  test("a chat with nobody else is not one", () => {
    expect(isNoReplyChat(chat({ participants: [] }))).toBe(false);
  });
});

describe("isUnsubscribeChat", () => {
  const to = (address: string) => [{ name: address, address }];
  const sent = (subject: string, body: string): Message => ({
    ...message(true),
    subject,
    body,
  });

  test("unsubscribe addresses", () => {
    for (const address of [
      "unsub-bb70314d.jqpj@bnc3.mailjet.com",
      "bounce+unsubscribe_6960cc@hicareer.com",
      "leave-cxn2u5zgth4unjtpxa2rej4f3i.510007@leave.crm.flypgs.com",
      "32.mrtvindlm5xucqkk@unsubscribe2.customer.io",
    ]) {
      expect(isUnsubscribeChat(chat({ participants: to(address) }))).toBe(true);
    }
    expect(isUnsubscribeChat(chat({ participants: to("leave@shop.sk") }))).toBe(false);
    expect(isUnsubscribeChat(chat({ participants: to("anna.unsworth@university.sk") }))).toBe(
      false,
    );
  });

  test("one-click requests from Gmail and Apple Mail", () => {
    const esputnik = to("complain@mta1.esputnik.com");
    expect(
      isUnsubscribeChat(
        chat({
          participants: esputnik,
          last: sent(
            "unsubscribe-334f7b20",
            "Apple Mail sent this email to unsubscribe from the message “unsubscribe-334f7b20”.",
          ),
        }),
      ),
    ).toBe(true);
    expect(
      isUnsubscribeChat(
        chat({
          participants: esputnik,
          last: sent("unsubscribe", "This message was automatically generated by Gmail."),
        }),
      ),
    ).toBe(true);
    expect(
      isUnsubscribeChat(
        chat({ participants: esputnik, last: sent("Hi", "How do I unsubscribe?") }),
      ),
    ).toBe(false);
  });
});
