import { useEffect, useState } from "react";
import { core } from "./core";
import { matchesQuery } from "./chat";
import type { Chat } from "../types";

/** Wait this long after the last keystroke before asking the core. */
export const SEARCH_DEBOUNCE_MS = 150;

/** Full-text search results for one query: chat ids by rank, and excerpts. */
export interface SearchResults {
  query: string;
  rank: Record<string, number>;
  snippets: Record<string, string>;
}

/** Runs the FTS5 search in the core; null without a core (browser preview) or on error. */
export async function fetchSearch(query: string): Promise<SearchResults | null> {
  const q = query.trim();
  if (!q) return null;
  try {
    const hits = await core.searchChats(q);
    const results: SearchResults = { query: q, rank: {}, snippets: {} };
    hits.forEach((h, i) => {
      results.rank[h.chatId] = i;
      if (h.snippet) results.snippets[h.chatId] = h.snippet;
    });
    return results;
  } catch {
    return null;
  }
}

/** The results if they belong to `query`; stale ones (still typing) are ignored. */
function current(query: string, results: SearchResults | null): SearchResults | null {
  return results && results.query === query.trim() ? results : null;
}

/**
 * Matches all cached mail via full-text search; until those results arrive (and in the
 * browser preview), the participants and newest message are checked locally.
 */
export function matchesSearch(chat: Chat, query: string, results: SearchResults | null): boolean {
  const r = current(query, results);
  return (r !== null && chat.id in r.rank) || matchesQuery(chat, query);
}

/** Best match first; chats only matched locally come after the ranked ones. */
export function searchOrder(
  query: string,
  results: SearchResults | null,
): ((a: Chat, b: Chat) => number) | null {
  const r = current(query, results);
  if (!r) return null;
  const rank = (c: Chat) => r.rank[c.id] ?? Number.MAX_SAFE_INTEGER;
  return (a, b) => rank(a) - rank(b);
}

export function snippetFor(
  chat: Chat,
  query: string,
  results: SearchResults | null,
): string | undefined {
  return current(query, results)?.snippets[chat.id];
}

/** Debounced full-text search for a text field. */
export function useChatSearch(query: string): SearchResults | null {
  const [results, setResults] = useState<SearchResults | null>(null);
  useEffect(() => {
    if (!query.trim()) return;
    let live = true;
    const timer = setTimeout(() => {
      void fetchSearch(query).then((r) => live && setResults(r));
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [query]);
  return results;
}
