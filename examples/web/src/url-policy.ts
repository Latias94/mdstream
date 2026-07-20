export type ClassifiedExternalUrl =
  | { readonly kind: "link"; readonly href: string }
  | { readonly kind: "inert"; readonly text: string };

export function classifyExternalUrl(destination: string): ClassifiedExternalUrl {
  try {
    const parsed = new URL(destination);
    if (parsed.protocol === "https:" || parsed.protocol === "http:") {
      return Object.freeze({ kind: "link", href: parsed.href });
    }
  } catch {
    // Invalid or relative destinations remain visible without becoming active.
  }
  return Object.freeze({ kind: "inert", text: destination });
}
