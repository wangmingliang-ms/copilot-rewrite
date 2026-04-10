/** Find the closing quote of a JSON string value.
 *  LLMs sometimes emit unescaped " inside string values (e.g. Chinese quotes "修改").
 *  We detect the real closing quote by checking what follows it:
 *  - Real closing quote is followed by JSON structural chars: , } ] or whitespace then these.
 *  - Fake mid-value quote is followed by more text content.
 *
 *  IMPORTANT: This function receives the string AFTER the opening quote of the value.
 *  e.g. for `"hello world",` it receives `hello world",` and should return the index of `"`. */
export function findClosingQuote(s: string): number {
  let lastCandidate = -1;
  for (let i = 0; i < s.length; i++) {
    if (s[i] === '\\') { i++; continue; } // skip escaped char
    if (s[i] === '"') {
      // Check what follows this quote (skip whitespace)
      let j = i + 1;
      while (j < s.length && (s[j] === ' ' || s[j] === '\t' || s[j] === '\r' || s[j] === '\n')) j++;
      const next = s[j];
      // Real closing quote: followed by , } ] : or end of string
      // (: is included because the next key starts with "key":)
      if (next === undefined || next === ',' || next === '}' || next === ']' || next === ':') {
        return i;
      }
      // A quote followed by another quote is likely the end of this value and start of next key
      // e.g. ...状态。""summary"... (missing comma between fields)
      if (next === '"') {
        return i;
      }
      // Otherwise it's likely an unescaped quote inside the value — skip it
      lastCandidate = i;
    }
  }
  // If no structural match found, return the last quote we saw (or -1)
  return lastCandidate;
}

/** Unescape a JSON string value — handles all standard JSON escape sequences. */
function unescapeJsonString(s: string): string {
  let result = "";
  for (let i = 0; i < s.length; i++) {
    if (s[i] === '\\' && i + 1 < s.length) {
      const next = s[i + 1];
      switch (next) {
        case '"': result += '"'; i++; break;
        case '\\': result += '\\'; i++; break;
        case '/': result += '/'; i++; break;
        case 'n': result += '\n'; i++; break;
        case 'r': result += '\r'; i++; break;
        case 't': result += '\t'; i++; break;
        case 'b': result += '\b'; i++; break;
        case 'f': result += '\f'; i++; break;
        case 'u': {
          // Unicode escape: \uXXXX
          const hex = s.substring(i + 2, i + 6);
          if (hex.length === 4 && /^[0-9a-fA-F]{4}$/.test(hex)) {
            result += String.fromCharCode(parseInt(hex, 16));
            i += 5; // skip \uXXXX
          } else {
            result += '\\u'; // malformed, keep as-is
            i++;
          }
          break;
        }
        default:
          // Unknown escape — keep as-is
          result += '\\' + next;
          i++;
          break;
      }
    } else {
      result += s[i];
    }
  }
  return result;
}

/** Extract a JSON string value for a given key from potentially partial JSON.
 *  Returns the unescaped string content, or null if the key is not found.
 *  Tolerant of: unescaped quotes in values, partial/streaming JSON, pretty-printed JSON. */
export function extractJsonStringValue(json: string, key: string): string | null {
  const marker = `"${key}"`;
  const idx = json.indexOf(marker);
  if (idx === -1) return null;
  const afterKey = json.substring(idx + marker.length);
  const valMatch = afterKey.match(/^\s*:\s*"/);
  if (!valMatch) return null;
  const valueStart = idx + marker.length + valMatch[0].length;
  let raw = json.substring(valueStart);
  const closingIdx = findClosingQuote(raw);
  if (closingIdx !== -1) {
    raw = raw.substring(0, closingIdx);
  }
  return unescapeJsonString(raw);
}

/** Strip code fences (```/~~~ with optional language tag) from LLM output.
 *  Tolerant of leading/trailing whitespace. */
export function stripCodeFences(text: string): string {
  return text.replace(/^\s*(?:```|~~~)\w*\s*\n?/i, "").replace(/\n?\s*(?:```|~~~)\s*$/, "");
}

/** Extract vocabulary array from potentially malformed JSON text.
 *  Uses regex to find individual {term, meaning} objects.
 *  Handles nested braces in values by using a simple brace-depth counter. */
export function extractVocabulary(text: string): { term: string; meaning: string }[] {
  const vocabulary: { term: string; meaning: string }[] = [];
  const vocabIdx = text.indexOf('"vocabulary"');
  if (vocabIdx === -1) return vocabulary;
  const afterVocab = text.substring(vocabIdx);
  const arrStart = afterVocab.indexOf("[");
  if (arrStart === -1) return vocabulary;
  const arrContent = afterVocab.substring(arrStart + 1);

  // Extract individual objects using brace-depth tracking instead of simple regex
  // This handles values that contain } characters
  let depth = 0;
  let objStart = -1;
  let inString = false;
  for (let i = 0; i < arrContent.length; i++) {
    const ch = arrContent[i];

    // Handle string literals (skip contents)
    if (ch === '\\' && inString) { i++; continue; }
    if (ch === '"') { inString = !inString; continue; }
    if (inString) continue;

    // Track brace depth
    if (ch === '{') {
      if (depth === 0) objStart = i;
      depth++;
    } else if (ch === '}') {
      depth--;
      if (depth === 0 && objStart !== -1) {
        const objStr = arrContent.substring(objStart, i + 1);
        try {
          const obj = JSON.parse(objStr);
          if (obj.term) {
            vocabulary.push({ term: String(obj.term || ""), meaning: String(obj.meaning || "") });
          }
        } catch { /* malformed object, skip */ }
        objStart = -1;
      }
    } else if (ch === ']' && depth === 0) {
      // End of vocabulary array
      break;
    }
  }
  return vocabulary;
}

export interface ParsedReadResult {
  readLayout: "simple" | "withVocab" | "withSummary" | "";
  readTranslation: string;
  readSummary: string;
  readVocabulary: { term: string; meaning: string }[];
}

/** Build a ParsedReadResult from extracted fields. */
function buildResult(
  translation: string,
  summary: string,
  vocabulary: { term: string; meaning: string }[],
): ParsedReadResult {
  const hasVocab = vocabulary.length > 0;
  const hasSummary = summary.trim().length > 0;
  const layout = hasSummary ? "withSummary" : hasVocab ? "withVocab" : "simple";
  return {
    readLayout: layout as ParsedReadResult["readLayout"],
    readTranslation: translation,
    readSummary: hasSummary ? summary : "",
    readVocabulary: vocabulary,
  };
}

/** Parse Read Mode LLM output with multiple fallback levels.
 *  Level 1: JSON.parse on cleaned text
 *  Level 2: JSON.parse after fixing literal newlines in string values
 *  Level 3: extractJsonStringValue (tolerant of unescaped quotes, malformed JSON) */
export function parseReadModeResult(text: string): ParsedReadResult | null {
  const cleaned = stripCodeFences(text);

  // Level 1: Try direct JSON.parse
  let parsed: Record<string, unknown> | null = null;
  try {
    parsed = JSON.parse(cleaned);
  } catch {
    // Level 2: Fix literal newlines/tabs inside JSON string values.
    // We track inString state — if this gets corrupted by unescaped quotes,
    // the parse will fail and we fall through to Level 3.
    let fixed = "";
    let inString = false;
    for (let i = 0; i < cleaned.length; i++) {
      const ch = cleaned[i];
      if (ch === '\\' && inString) {
        fixed += ch + (cleaned[i + 1] || "");
        i++;
        continue;
      }
      if (ch === '"') {
        inString = !inString;
        fixed += ch;
        continue;
      }
      if (inString && (ch === '\n' || ch === '\r' || ch === '\t')) {
        if (ch === '\r' && cleaned[i + 1] === '\n') i++;
        fixed += ch === '\t' ? "\\t" : "\\n";
        continue;
      }
      fixed += ch;
    }
    try {
      parsed = JSON.parse(fixed);
    } catch {
      // Level 3: Extract values individually (tolerant of unescaped quotes)
      const translation = extractJsonStringValue(cleaned, "translation");
      if (translation) {
        const summary = extractJsonStringValue(cleaned, "summary") || "";
        const vocabulary = extractVocabulary(cleaned);
        return buildResult(translation, summary, vocabulary);
      }
    }
  }

  if (parsed && parsed.translation) {
    const vocab = Array.isArray(parsed.vocabulary) ? parsed.vocabulary : [];
    const validVocab = vocab.filter(
      (v: unknown): v is { term: string; meaning: string } =>
        typeof v === "object" && v !== null && "term" in v
    );
    const summary = typeof parsed.summary === "string" ? parsed.summary : "";
    return buildResult(String(parsed.translation), summary, validVocab);
  }

  return null;
}
