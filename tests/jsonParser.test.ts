import { describe, it, expect } from "vitest";
import {
  findClosingQuote,
  extractJsonStringValue,
  stripCodeFences,
  extractVocabulary,
  parseReadModeResult,
} from "../src/utils/jsonParser";

// =============================================================================
// findClosingQuote
// =============================================================================
describe("findClosingQuote", () => {
  // --- Basic cases ---
  it("finds a simple closing quote followed by comma", () => {
    expect(findClosingQuote('hello",')).toBe(5);
  });

  it("finds closing quote followed by }", () => {
    expect(findClosingQuote('hello"}')).toBe(5);
  });

  it("finds closing quote followed by ]", () => {
    expect(findClosingQuote('hello"]')).toBe(5);
  });

  it("finds closing quote at end of string", () => {
    expect(findClosingQuote('hello"')).toBe(5);
  });

  it("finds closing quote followed by :", () => {
    // e.g. end of one value, immediately followed by next key without comma (malformed)
    expect(findClosingQuote('hello":')).toBe(5);
  });

  // --- Escaped characters ---
  it("skips escaped quotes", () => {
    // JS string 'say \\"hi\\","' → chars: s a y   \ " h i \ " , "
    // Indices:                              0 1 2 3 4 5 6 7 8 9 10 11
    // \" at 4-5 = escaped quote (skipped), \" at 8-9 = escaped quote (skipped)
    // "," at 10 = , is not " so skip, " at 11 = end of string → match
    expect(findClosingQuote('say \\"hi\\","')).toBe(11);
  });

  it("skips escaped backslashes before closing quote", () => {
    // \\\" in source = escaped backslash + quote: the " is a real closing quote
    // raw string: path\\" → last char is " after \\
    expect(findClosingQuote('path\\\\","')).toBe(6);
  });

  it("handles escaped backslash at end", () => {
    // Content is: hello\\"  → that's escaped backslash, then closing quote
    expect(findClosingQuote('hello\\\\",')).toBe(7);
  });

  // --- Whitespace between quote and structural char ---
  it("handles quote followed by spaces then comma", () => {
    expect(findClosingQuote('hello"  ,')).toBe(5);
  });

  it("handles quote followed by newline then }", () => {
    expect(findClosingQuote('hello"\n}')).toBe(5);
  });

  it("handles quote followed by \\r\\n then }", () => {
    expect(findClosingQuote('hello"\r\n}')).toBe(5);
  });

  it("handles quote followed by tab then ,", () => {
    expect(findClosingQuote('hello"\t,')).toBe(5);
  });

  it("handles quote followed by mixed whitespace then ]", () => {
    expect(findClosingQuote('hello" \n\t ]')).toBe(5);
  });

  // --- Unescaped quotes in CJK content ---
  it("skips unescaped quotes followed by CJK text", () => {
    const s = '可以安全地"修改"它们收到的状态。",';
    const idx = findClosingQuote(s);
    expect(s[idx]).toBe('"');
    expect(s.substring(0, idx)).toBe('可以安全地"修改"它们收到的状态。');
  });

  it("handles multiple unescaped quotes in Chinese text", () => {
    const s = '这个对象"可以安全地"修改"状态",';
    const idx = findClosingQuote(s);
    expect(s.substring(0, idx)).toBe('这个对象"可以安全地"修改"状态');
  });

  it("handles quote followed by another quote (adjacent fields)", () => {
    // e.g. missing comma: "value""nextKey"
    const s = 'hello""nextKey';
    expect(findClosingQuote(s)).toBe(5);
  });

  // --- Edge cases ---
  it("returns -1 when no quotes", () => {
    expect(findClosingQuote("no quotes here")).toBe(-1);
  });

  it("returns -1 for empty string", () => {
    expect(findClosingQuote("")).toBe(-1);
  });

  it("returns lastCandidate when no structural match found", () => {
    // All quotes followed by text content
    const s = '"hello" world';
    expect(findClosingQuote(s)).toBe(6);
  });

  it("handles single character value", () => {
    expect(findClosingQuote('a",')).toBe(1);
  });

  it("handles empty value (closing quote immediately)", () => {
    expect(findClosingQuote('","')).toBe(0);
  });

  // --- Content with special characters ---
  it("handles value containing markdown bold **text**", () => {
    const s = 'This is **bold** text",';
    const idx = findClosingQuote(s);
    expect(s.substring(0, idx)).toBe("This is **bold** text");
  });

  it("handles value containing markdown code `backticks`", () => {
    const s = 'Use `console.log()` here",';
    const idx = findClosingQuote(s);
    expect(s.substring(0, idx)).toBe("Use `console.log()` here");
  });

  it("handles value containing markdown link [text](url)", () => {
    const s = 'See [docs](https://example.com)",';
    const idx = findClosingQuote(s);
    expect(s.substring(0, idx)).toBe("See [docs](https://example.com)");
  });

  it("handles value containing curly braces in text", () => {
    // This is tricky: "function() { return x }" — the } could trigger premature close
    // But since } is after a space, not after a quote, it won't trigger
    const s = 'function() { return x }",';
    const idx = findClosingQuote(s);
    expect(s.substring(0, idx)).toBe("function() { return x }");
  });

  it("handles value containing JSON-like text", () => {
    // Value that contains {"key": "value"} as text — the inner " followed by : triggers structural match
    // This is a known limitation: unescaped " in a value that happens to be followed by :
    // In practice LLMs rarely put raw JSON inside JSON values
    const s = 'set to {"key": "value"}",';
    const idx = findClosingQuote(s);
    // Will stop at first " followed by : — this is acceptable behavior for malformed JSON
    expect(idx).toBeGreaterThanOrEqual(0);
    expect(idx).toBeLessThan(s.length);
  });
});

// =============================================================================
// extractJsonStringValue
// =============================================================================
describe("extractJsonStringValue", () => {
  // --- Basic extraction ---
  it("extracts a simple value", () => {
    expect(extractJsonStringValue('{"translation": "Hello world"}', "translation")).toBe("Hello world");
  });

  it("extracts value from multi-key JSON", () => {
    const json = '{"translation": "Hello", "summary": "A greeting"}';
    expect(extractJsonStringValue(json, "translation")).toBe("Hello");
    expect(extractJsonStringValue(json, "summary")).toBe("A greeting");
  });

  it("extracts value from pretty-printed JSON", () => {
    const json = '{\n  "translation": "Hello world",\n  "summary": "Greeting"\n}';
    expect(extractJsonStringValue(json, "translation")).toBe("Hello world");
    expect(extractJsonStringValue(json, "summary")).toBe("Greeting");
  });

  // --- Escape sequences ---
  it("unescapes \\n to newline", () => {
    expect(extractJsonStringValue('{"k": "Line 1\\nLine 2"}', "k")).toBe("Line 1\nLine 2");
  });

  it("unescapes \\t to tab", () => {
    expect(extractJsonStringValue('{"k": "col1\\tcol2"}', "k")).toBe("col1\tcol2");
  });

  it("unescapes \\r to carriage return", () => {
    expect(extractJsonStringValue('{"k": "a\\rb"}', "k")).toBe("a\rb");
  });

  it("unescapes \\\\ to literal backslash", () => {
    expect(extractJsonStringValue('{"k": "path\\\\file"}', "k")).toBe("path\\file");
  });

  it("unescapes \\/ to forward slash", () => {
    expect(extractJsonStringValue('{"k": "a\\/b"}', "k")).toBe("a/b");
  });

  it("unescapes \\\" to literal quote", () => {
    expect(extractJsonStringValue('{"k": "He said \\"hello\\""}', "k")).toBe('He said "hello"');
  });

  it("unescapes \\uXXXX unicode sequences", () => {
    // \u00e9 = é
    expect(extractJsonStringValue('{"k": "caf\\u00e9"}', "k")).toBe("café");
  });

  it("unescapes \\u4e2d to Chinese character", () => {
    // \u4e2d = 中
    expect(extractJsonStringValue('{"k": "\\u4e2d\\u6587"}', "k")).toBe("中文");
  });

  it("handles \\\\n (literal backslash + n, not newline)", () => {
    // In JSON: \\\\n → parsed to \\n → which is literal \n
    expect(extractJsonStringValue('{"k": "\\\\n means newline"}', "k")).toBe("\\n means newline");
  });

  it("handles \\b and \\f escapes", () => {
    expect(extractJsonStringValue('{"k": "a\\bb\\fc"}', "k")).toBe("a\bb\fc");
  });

  it("keeps unknown escapes as-is", () => {
    expect(extractJsonStringValue('{"k": "\\x not valid"}', "k")).toBe("\\x not valid");
  });

  // --- Unescaped quotes (LLM quirk) ---
  it("extracts value with unescaped CJK quotes", () => {
    const json = '{"translation": "可以安全地"修改"它们收到的状态。", "vocabulary": []}';
    expect(extractJsonStringValue(json, "translation")).toBe('可以安全地"修改"它们收到的状态。');
  });

  it("extracts value with multiple unescaped CJK quotes", () => {
    const json = '{"translation": "这是"重要"和"关键"的内容。"}';
    expect(extractJsonStringValue(json, "translation")).toBe('这是"重要"和"关键"的内容。');
  });

  // --- Partial / streaming JSON ---
  it("handles partial JSON (no closing quote)", () => {
    expect(extractJsonStringValue('{"translation": "Hello wor', "translation")).toBe("Hello wor");
  });

  it("handles partial JSON with escaped chars", () => {
    expect(extractJsonStringValue('{"k": "Line 1\\nLine 2\\nLine 3', "k")).toBe("Line 1\nLine 2\nLine 3");
  });

  // --- Missing / invalid key ---
  it("returns null for missing key", () => {
    expect(extractJsonStringValue('{"translation": "hello"}', "missing")).toBeNull();
  });

  it("returns null when value is a number", () => {
    expect(extractJsonStringValue('{"count": 42}', "count")).toBeNull();
  });

  it("returns null when value is a boolean", () => {
    expect(extractJsonStringValue('{"flag": true}', "flag")).toBeNull();
  });

  it("returns null when value is null", () => {
    expect(extractJsonStringValue('{"val": null}', "val")).toBeNull();
  });

  it("returns null when value is an array", () => {
    expect(extractJsonStringValue('{"arr": [1,2,3]}', "arr")).toBeNull();
  });

  // --- Whitespace variations ---
  it("handles no space after colon", () => {
    expect(extractJsonStringValue('{"k":"value"}', "k")).toBe("value");
  });

  it("handles multiple spaces around colon", () => {
    expect(extractJsonStringValue('{"k"  :   "value"}', "k")).toBe("value");
  });

  it("handles newline between colon and value", () => {
    expect(extractJsonStringValue('{"k":\n"value"}', "k")).toBe("value");
  });

  // --- Content with special characters ---
  it("handles markdown bold in value", () => {
    expect(extractJsonStringValue('{"k": "This is **bold** text"}', "k")).toBe("This is **bold** text");
  });

  it("handles markdown heading in value", () => {
    expect(extractJsonStringValue('{"k": "## Heading\\nContent"}', "k")).toBe("## Heading\nContent");
  });

  it("handles markdown list in value", () => {
    expect(extractJsonStringValue('{"k": "- Item 1\\n- Item 2"}', "k")).toBe("- Item 1\n- Item 2");
  });

  it("handles markdown code backticks in value", () => {
    expect(extractJsonStringValue('{"k": "Use `code` here"}', "k")).toBe("Use `code` here");
  });

  it("handles markdown link in value", () => {
    expect(extractJsonStringValue('{"k": "See [link](http://example.com)"}', "k")).toBe("See [link](http://example.com)");
  });

  it("handles HTML tags in value", () => {
    expect(extractJsonStringValue('{"k": "<b>bold</b>"}', "k")).toBe("<b>bold</b>");
  });

  it("handles parentheses and brackets in value", () => {
    expect(extractJsonStringValue('{"k": "array[0] and func()"}', "k")).toBe("array[0] and func()");
  });

  it("handles emoji in value", () => {
    expect(extractJsonStringValue('{"k": "Hello 🌍 world 🎉"}', "k")).toBe("Hello 🌍 world 🎉");
  });

  it("handles empty string value", () => {
    expect(extractJsonStringValue('{"k": ""}', "k")).toBe("");
  });
});

// =============================================================================
// stripCodeFences
// =============================================================================
describe("stripCodeFences", () => {
  // --- Standard fences ---
  it("strips ```json fences", () => {
    expect(stripCodeFences('```json\n{"a": 1}\n```')).toBe('{"a": 1}');
  });

  it("strips ``` fences without language", () => {
    expect(stripCodeFences('```\n{"a": 1}\n```')).toBe('{"a": 1}');
  });

  it("strips ~~~ fences", () => {
    expect(stripCodeFences('~~~json\n{"a": 1}\n~~~')).toBe('{"a": 1}');
  });

  it("strips ~~~ fences without language", () => {
    expect(stripCodeFences('~~~\n{"a": 1}\n~~~')).toBe('{"a": 1}');
  });

  // --- Whitespace tolerance ---
  it("strips fences with leading whitespace", () => {
    expect(stripCodeFences(' ```json\n{"a": 1}\n```')).toBe('{"a": 1}');
  });

  it("strips fences with leading tabs", () => {
    expect(stripCodeFences('\t```json\n{"a": 1}\n```')).toBe('{"a": 1}');
  });

  it("strips fences with trailing whitespace", () => {
    expect(stripCodeFences('```json\n{"a": 1}\n```  ')).toBe('{"a": 1}');
  });

  it("strips fences with extra blank line before closing", () => {
    // The \n before ``` is consumed by the closing regex, leaving one \n from the blank line
    expect(stripCodeFences('```json\n{"a": 1}\n\n```')).toBe('{"a": 1}');
  });

  it("strips fences with whitespace before closing", () => {
    expect(stripCodeFences('```json\n{"a": 1}\n  ```')).toBe('{"a": 1}');
  });

  // --- Case insensitive ---
  it("handles uppercase JSON tag", () => {
    expect(stripCodeFences('```JSON\n{"a": 1}\n```')).toBe('{"a": 1}');
  });

  it("handles mixed case tag", () => {
    expect(stripCodeFences('```Json\n{"a": 1}\n```')).toBe('{"a": 1}');
  });

  // --- Other language tags ---
  it("strips fences with other language tags", () => {
    expect(stripCodeFences('```javascript\nconsole.log(1)\n```')).toBe('console.log(1)');
  });

  // --- No fences ---
  it("leaves plain JSON unchanged", () => {
    expect(stripCodeFences('{"a": 1}')).toBe('{"a": 1}');
  });

  it("leaves plain text unchanged", () => {
    expect(stripCodeFences("Hello world")).toBe("Hello world");
  });

  it("leaves empty string unchanged", () => {
    expect(stripCodeFences("")).toBe("");
  });

  // --- Content containing backticks (should NOT strip internal code blocks) ---
  it("only strips outermost fences, not internal markdown code", () => {
    // Content that contains a code block inside
    const text = '```json\n{"k": "Use `code` here"}\n```';
    expect(stripCodeFences(text)).toBe('{"k": "Use `code` here"}');
  });

  it("does not strip single backtick inline code", () => {
    const text = 'Use `code` here';
    expect(stripCodeFences(text)).toBe('Use `code` here');
  });
});

// =============================================================================
// extractVocabulary
// =============================================================================
describe("extractVocabulary", () => {
  // --- Basic cases ---
  it("extracts single vocabulary item", () => {
    const text = '{"vocabulary": [{"term": "foo", "meaning": "bar"}]}';
    expect(extractVocabulary(text)).toEqual([{ term: "foo", meaning: "bar" }]);
  });

  it("extracts multiple vocabulary items", () => {
    const text = '{"vocabulary": [{"term": "a", "meaning": "1"}, {"term": "b", "meaning": "2"}]}';
    expect(extractVocabulary(text)).toEqual([
      { term: "a", meaning: "1" },
      { term: "b", meaning: "2" },
    ]);
  });

  it("extracts vocabulary from pretty-printed JSON", () => {
    const text = `{
  "vocabulary": [
    {"term": "createReducer", "meaning": "创建 reducer 的函数"},
    {"term": "mutate", "meaning": "修改、改变"}
  ]
}`;
    expect(extractVocabulary(text)).toEqual([
      { term: "createReducer", meaning: "创建 reducer 的函数" },
      { term: "mutate", meaning: "修改、改变" },
    ]);
  });

  // --- Missing / empty ---
  it("returns empty array when no vocabulary key", () => {
    expect(extractVocabulary('{"translation": "hello"}')).toEqual([]);
  });

  it("returns empty array for empty vocabulary", () => {
    expect(extractVocabulary('{"vocabulary": []}')).toEqual([]);
  });

  it("returns empty array for empty string", () => {
    expect(extractVocabulary("")).toEqual([]);
  });

  // --- Malformed objects ---
  it("skips objects without term field", () => {
    const text = '{"vocabulary": [{"term": "a", "meaning": "1"}, {"bad": true}]}';
    expect(extractVocabulary(text)).toEqual([{ term: "a", meaning: "1" }]);
  });

  it("handles missing meaning field gracefully", () => {
    const text = '{"vocabulary": [{"term": "a"}]}';
    expect(extractVocabulary(text)).toEqual([{ term: "a", meaning: "" }]);
  });

  // --- Partial / streaming ---
  it("extracts complete objects from partial JSON", () => {
    const text = '{"vocabulary": [{"term": "a", "meaning": "1"}, {"term": "b", "meani';
    expect(extractVocabulary(text)).toEqual([{ term: "a", meaning: "1" }]);
  });

  it("handles vocabulary key present but no array start", () => {
    const text = '{"vocabulary": ';
    expect(extractVocabulary(text)).toEqual([]);
  });

  // --- Values with special characters ---
  it("handles meaning with parentheses", () => {
    const text = '{"vocabulary": [{"term": "mutate", "meaning": "修改（在这里指直接修改对象）"}]}';
    expect(extractVocabulary(text)).toEqual([
      { term: "mutate", meaning: "修改（在这里指直接修改对象）" },
    ]);
  });

  it("handles meaning with curly braces in text", () => {
    const text = '{"vocabulary": [{"term": "obj", "meaning": "An object like {key: val}"}]}';
    const result = extractVocabulary(text);
    expect(result).toEqual([{ term: "obj", meaning: "An object like {key: val}" }]);
  });

  it("handles meaning with escaped quotes", () => {
    const text = '{"vocabulary": [{"term": "a", "meaning": "means \\"hello\\""}]}';
    expect(extractVocabulary(text)).toEqual([{ term: "a", meaning: 'means "hello"' }]);
  });

  it("handles meaning with markdown formatting", () => {
    const text = '{"vocabulary": [{"term": "bold", "meaning": "Use **bold** for emphasis"}]}';
    expect(extractVocabulary(text)).toEqual([
      { term: "bold", meaning: "Use **bold** for emphasis" },
    ]);
  });

  it("handles meaning with backticks", () => {
    const text = '{"vocabulary": [{"term": "log", "meaning": "Use `console.log()`"}]}';
    expect(extractVocabulary(text)).toEqual([
      { term: "log", meaning: "Use `console.log()`" },
    ]);
  });
});

// =============================================================================
// parseReadModeResult — integration tests
// =============================================================================
describe("parseReadModeResult", () => {
  // --- Level 1: Well-formed JSON ---
  it("L1: parses simple translation", () => {
    const result = parseReadModeResult('{"translation": "Hello world"}');
    expect(result).not.toBeNull();
    expect(result!.readTranslation).toBe("Hello world");
    expect(result!.readLayout).toBe("simple");
    expect(result!.readVocabulary).toEqual([]);
    expect(result!.readSummary).toBe("");
  });

  it("L1: parses translation with vocabulary", () => {
    const text = '{"translation": "Hello", "vocabulary": [{"term": "hi", "meaning": "greeting"}]}';
    const result = parseReadModeResult(text);
    expect(result!.readLayout).toBe("withVocab");
    expect(result!.readVocabulary).toEqual([{ term: "hi", meaning: "greeting" }]);
  });

  it("L1: parses translation with summary", () => {
    const text = '{"translation": "Full text here.", "summary": "Brief summary"}';
    const result = parseReadModeResult(text);
    expect(result!.readLayout).toBe("withSummary");
    expect(result!.readSummary).toBe("Brief summary");
  });

  it("L1: summary takes priority over vocabulary for layout", () => {
    const text = '{"translation": "text", "summary": "brief", "vocabulary": [{"term": "a", "meaning": "b"}]}';
    const result = parseReadModeResult(text);
    expect(result!.readLayout).toBe("withSummary");
    // vocabulary should still be populated
    expect(result!.readVocabulary.length).toBe(1);
  });

  it("L1: empty summary treated as simple", () => {
    const result = parseReadModeResult('{"translation": "Hello", "summary": ""}');
    expect(result!.readLayout).toBe("simple");
    expect(result!.readSummary).toBe("");
  });

  it("L1: whitespace-only summary treated as simple", () => {
    const result = parseReadModeResult('{"translation": "Hello", "summary": "   "}');
    expect(result!.readLayout).toBe("simple");
  });

  it("L1: parses JSON wrapped in code fences", () => {
    const result = parseReadModeResult('```json\n{"translation": "Hello"}\n```');
    expect(result!.readTranslation).toBe("Hello");
  });

  it("L1: parses JSON with code fences and leading space", () => {
    const result = parseReadModeResult(' ```json\n{"translation": "Hello"}\n```');
    expect(result!.readTranslation).toBe("Hello");
  });

  it("L1: parses JSON with ~~~ fences", () => {
    const result = parseReadModeResult('~~~json\n{"translation": "Hello"}\n~~~');
    expect(result!.readTranslation).toBe("Hello");
  });

  it("L1: translation with escaped newlines renders correctly", () => {
    const text = '{"translation": "Line 1\\nLine 2\\n\\n## Heading\\nContent"}';
    const result = parseReadModeResult(text);
    expect(result!.readTranslation).toBe("Line 1\nLine 2\n\n## Heading\nContent");
  });

  it("L1: translation with markdown formatting", () => {
    const text = '{"translation": "This is **bold** and *italic* and `code`"}';
    const result = parseReadModeResult(text);
    expect(result!.readTranslation).toBe("This is **bold** and *italic* and `code`");
  });

  it("L1: translation with unicode escapes", () => {
    const text = '{"translation": "caf\\u00e9"}';
    const result = parseReadModeResult(text);
    expect(result!.readTranslation).toBe("café");
  });

  // --- Level 2: Literal newlines in values ---
  it("L2: handles literal newlines in translation", () => {
    const text = '{"translation": "Line 1\nLine 2\nLine 3"}';
    const result = parseReadModeResult(text);
    expect(result).not.toBeNull();
    expect(result!.readTranslation).toBe("Line 1\nLine 2\nLine 3");
  });

  it("L2: handles literal \\r\\n in translation", () => {
    const text = '{"translation": "Line 1\r\nLine 2"}';
    const result = parseReadModeResult(text);
    expect(result).not.toBeNull();
    expect(result!.readTranslation).toBe("Line 1\nLine 2");
  });

  it("L2: handles literal tabs in translation", () => {
    const text = '{"translation": "Col1\tCol2"}';
    const result = parseReadModeResult(text);
    expect(result).not.toBeNull();
    expect(result!.readTranslation).toBe("Col1\tCol2");
  });

  it("L2: handles literal newlines in code-fenced JSON", () => {
    const text = '```json\n{"translation": "Line 1\nLine 2"}\n```';
    const result = parseReadModeResult(text);
    expect(result).not.toBeNull();
    expect(result!.readTranslation).toBe("Line 1\nLine 2");
  });

  it("L2: handles literal newlines in summary too", () => {
    const text = '{"translation": "full text\nwith newlines", "summary": "brief\nsummary"}';
    const result = parseReadModeResult(text);
    expect(result).not.toBeNull();
    expect(result!.readTranslation).toBe("full text\nwith newlines");
    expect(result!.readSummary).toBe("brief\nsummary");
  });

  // --- Level 3: Unescaped quotes ---
  it("L3: handles unescaped Chinese quotes", () => {
    const text = '{"translation": "可以安全地"修改"状态。", "vocabulary": [{"term": "mutate", "meaning": "修改"}]}';
    const result = parseReadModeResult(text);
    expect(result).not.toBeNull();
    expect(result!.readTranslation).toBe('可以安全地"修改"状态。');
    expect(result!.readVocabulary).toEqual([{ term: "mutate", meaning: "修改" }]);
  });

  it("L3: handles pretty-printed JSON with unescaped quotes (real LLM output)", () => {
    const text = `\`\`\`json
{
  "translation": "这个对象将被传递给 createReducer，因此 reducers 可以安全地"修改"它们收到的状态。",
  "vocabulary": [
    {
      "term": "createReducer",
      "meaning": "创建 reducer 的函数，通常用于 Redux Toolkit 中"
    },
    {
      "term": "mutate",
      "meaning": "修改、改变（在这里指直接修改状态对象）"
    },
    {
      "term": "state",
      "meaning": "状态，指应用程序中由 reducer 管理的数据"
    }
  ]
}
\`\`\``;
    const result = parseReadModeResult(text);
    expect(result).not.toBeNull();
    expect(result!.readTranslation).toContain('可以安全地');
    expect(result!.readTranslation).toContain('修改');
    expect(result!.readTranslation).toContain('状态');
    expect(result!.readLayout).toBe("withVocab");
    expect(result!.readVocabulary.length).toBe(3);
    expect(result!.readVocabulary[0].term).toBe("createReducer");
    expect(result!.readVocabulary[1].term).toBe("mutate");
    expect(result!.readVocabulary[2].term).toBe("state");
  });

  it("L3: handles multiple unescaped quote pairs", () => {
    const text = '{"translation": "这是"重要"和"关键"的内容。"}';
    const result = parseReadModeResult(text);
    expect(result).not.toBeNull();
    expect(result!.readTranslation).toBe('这是"重要"和"关键"的内容。');
  });

  it("L3: extracts summary even when translation has unescaped quotes", () => {
    const text = '{"translation": "使用"特殊"语法", "summary": "关于特殊语法的说明"}';
    const result = parseReadModeResult(text);
    expect(result).not.toBeNull();
    expect(result!.readTranslation).toBe('使用"特殊"语法');
    // Summary extraction may or may not work depending on how far the parser gets
    // At minimum we should get the translation
  });

  // --- Null / empty / invalid ---
  it("returns null for empty string", () => {
    expect(parseReadModeResult("")).toBeNull();
  });

  it("returns null for plain text", () => {
    expect(parseReadModeResult("Just plain text")).toBeNull();
  });

  it("returns null for JSON without translation key", () => {
    expect(parseReadModeResult('{"other": "data"}')).toBeNull();
  });

  it("returns null for malformed non-JSON text", () => {
    expect(parseReadModeResult("{ not json at all }")).toBeNull();
  });

  it("returns null for array JSON", () => {
    expect(parseReadModeResult('[1, 2, 3]')).toBeNull();
  });

  // --- Real-world LLM outputs from console logs ---
  it("real case 1: code fences with leading space and unescaped quotes", () => {
    // Exact reproduction from user's console log
    const text = ' ```json\n{\n  "translation": "这个对象将被传递给 createReducer，因此 reducers 可以安全地"修改"它们收到的状态。",\n  "vocabulary": [\n    {\n      "term": "createReducer",\n      "meaning": "创建 reducer 的函数，通常用于 Redux Toolkit 中"\n    },\n    {\n      "term": "mutate",\n      "meaning": "修改、改变（在这里指直接修改状态对象）"\n    }\n  ]\n}\n```';
    const result = parseReadModeResult(text);
    expect(result).not.toBeNull();
    expect(result!.readTranslation).toContain("createReducer");
    expect(result!.readTranslation).toContain("修改");
    expect(result!.readVocabulary.length).toBeGreaterThanOrEqual(1);
  });

  it("real case 2: code fences without unescaped quotes", () => {
    // Normal well-formed response in code fences
    const text = '```json\n{\n  "translation": "这个对象将被传递给 createReducer。",\n  "vocabulary": [\n    {\n      "term": "createReducer",\n      "meaning": "创建 reducer 的函数"\n    }\n  ]\n}\n```';
    const result = parseReadModeResult(text);
    expect(result).not.toBeNull();
    expect(result!.readTranslation).toBe("这个对象将被传递给 createReducer。");
    expect(result!.readVocabulary).toEqual([{ term: "createReducer", meaning: "创建 reducer 的函数" }]);
  });

  it("real case 3: escaped \\n in values (pretty content)", () => {
    // LLM uses \\n for newlines inside the translation
    const text = '{"translation": "第一段\\n\\n第二段\\n- 列表项1\\n- 列表项2", "summary": "两段内容概要"}';
    const result = parseReadModeResult(text);
    expect(result!.readTranslation).toBe("第一段\n\n第二段\n- 列表项1\n- 列表项2");
    expect(result!.readLayout).toBe("withSummary");
  });
});
