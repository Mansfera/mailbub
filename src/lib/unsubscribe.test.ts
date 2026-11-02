import { describe, expect, test } from "bun:test";
import { findHtmlLink, findTextLink, unsubscribeAction } from "./unsubscribe";

const mail = (patch: Partial<Parameters<typeof unsubscribeAction>[0]> = {}) => ({
  outgoing: false,
  body: "",
  html: null,
  quoted: undefined,
  unsubscribe: null,
  ...patch,
});

describe("findHtmlLink", () => {
  test("Mailchimp footer, link text in a span", () => {
    const html = `<!--[if mso]><a href="https://x.com/mso">unsubscribe</a><![endif]-->
      <a href="https://company.us6.list-manage.com/profile?u=1&amp;id=2">
        <span style="font-size: 11px">update your preferences</span></a> or
      <a href="https://company.us6.list-manage.com/unsubscribe?u=1&amp;id=2"><span>unsubscribe</span></a>`;
    expect(findHtmlLink(html)).toBe("https://company.us6.list-manage.com/unsubscribe?u=1&id=2");
  });

  test("link text in other languages, behind a click tracker", () => {
    const cases: [string, string][] = [
      ["Odhlásiť sa z odberu", "sk"],
      ["Відписатися від розсилки", "uk"],
      ["Newsletter abbestellen", "de"],
      ["Se désinscrire", "fr"],
      ["Darse de baja", "es"],
      ["Wypisz się", "pl"],
      ["配信停止はこちら", "ja"],
      ["Отписаться", "ru"],
    ];
    for (const [text, lang] of cases) {
      const html = `<a href="https://t.esp.com/c/${lang}">Shop</a> <a href='https://t.esp.com/c/9'>${text}</a>`;
      expect(findHtmlLink(html), lang).toBe("https://t.esp.com/c/9");
    }
  });

  test("by address when the text says nothing", () => {
    const html = `<a href="https://shop.sk/">Shop</a><a href="https://shop.sk/newsletter/optout?id=1">here</a>`;
    expect(findHtmlLink(html)).toBe("https://shop.sk/newsletter/optout?id=1");
  });

  test("ignores mailto and javascript, and plain links", () => {
    expect(findHtmlLink(`<a href="javascript:x()">unsubscribe</a>`)).toBeNull();
    expect(findHtmlLink(`<a href="https://shop.sk/sale">Sale</a>`)).toBeNull();
    expect(findHtmlLink(`<a href="https://substack.com/p/1">Read</a>`)).toBeNull();
  });
});

describe("findTextLink", () => {
  test("label before the URL", () => {
    expect(findTextLink("Bye\n\nunsubscribe (https://x.com/u?a=1)")).toBe("https://x.com/u?a=1");
    expect(findTextLink("Ak sa chcete odhlásiť, kliknite sem: https://x.sk/o.")).toBe(
      "https://x.sk/o",
    );
  });

  test("a label in an earlier paragraph does not count", () => {
    expect(findTextLink("You can unsubscribe any time.\n\nNew: https://x.com/news")).toBeNull();
  });
});

describe("unsubscribeAction", () => {
  test("one-click header first", () => {
    const m = mail({
      unsubscribe: { url: "https://x.com/1c", mailto: "mailto:u@x.com", oneClick: true },
      html: `<a href="https://x.com/page">Unsubscribe</a>`,
    });
    expect(unsubscribeAction(m, { links: true })).toEqual({
      kind: "oneClick",
      url: "https://x.com/1c",
    });
  });

  test("footer link only for automated mail, mailto last", () => {
    const m = mail({
      unsubscribe: { url: null, mailto: "mailto:u@x.com", oneClick: false },
      html: `<a href="https://x.com/page">Unsubscribe</a>`,
    });
    expect(unsubscribeAction(m, { links: true })).toEqual({
      kind: "web",
      url: "https://x.com/page",
    });
    expect(unsubscribeAction(m, { links: false })).toEqual({ kind: "mail", url: "mailto:u@x.com" });
    expect(unsubscribeAction(mail({ html: m.html }), { links: false })).toBeNull();
  });

  test("never for your own mail", () => {
    const m = mail({
      outgoing: true,
      unsubscribe: { url: "https://x.com/1c", mailto: null, oneClick: true },
    });
    expect(unsubscribeAction(m, { links: true })).toBeNull();
  });
});
