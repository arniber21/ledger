import { drainSse, finishSse } from "./reduce";
import type { AuthorEvent } from "./types";

/**
 * Posts a policy sentence and reads `ledger-spec`'s authoring frames.
 *
 * `overrideCoverage` asks the pipeline to arm a Same match that left a clause
 * uncovered or unknown. It does not invent a verdict.
 *
 * `VITE_SPEC_AUTHOR_URL` is that stream. When it is unset, the UI posts to
 * `/api/specs/author`. An empty value leaves authoring disconnected. A failed
 * request is shown as a failure. This module does not call Gemini or Jev and
 * does not fill in a draft, a verdict, or a score.
 */
export function authorUrl(): string | undefined {
  const url = import.meta.env.VITE_SPEC_AUTHOR_URL;
  if (url === "") return undefined;
  if (url && url.length > 0) return url;
  return "/api/specs/author";
}

export async function streamAuthoring(
  sentence: string,
  onEvent: (event: AuthorEvent) => void,
  signal: AbortSignal,
  overrideCoverage = false,
): Promise<void> {
  const url = authorUrl();
  if (!url) {
    throw new Error("Spec authoring is not connected.");
  }

  let response: Response;
  try {
    response = await fetch(url, {
      method: "POST",
      headers: {
        accept: "text/event-stream",
        "content-type": "application/json",
      },
      body: JSON.stringify({ sentence, override: overrideCoverage }),
      signal,
    });
  } catch (error) {
    if (isAbort(error)) throw error;
    throw new Error("The authoring stream could not be reached.");
  }

  if (!response.ok) {
    throw new Error(`The authoring stream returned ${response.status}.`);
  }
  if (!response.body) {
    throw new Error("The authoring stream returned an empty body.");
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let saw = false;
  const emit = (event: AuthorEvent) => {
    saw = true;
    onEvent(event);
  };

  while (true) {
    const { done, value } = await reader.read();
    buffer += decoder.decode(value ?? new Uint8Array(), { stream: !done });
    const drained = drainSse(buffer);
    buffer = drained.rest;
    for (const event of drained.events) emit(event);
    if (done) {
      for (const event of finishSse(buffer)) emit(event);
      break;
    }
  }

  if (!saw) {
    throw new Error("The authoring stream did not send any steps.");
  }
}

function isAbort(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError";
}
