import { describe, it, expect } from "vitest";
import {
  parseReadModeSeparator,
  parseVocabularyLines,
} from "../src/utils/jsonParser";

// =============================================================================
// parseVocabularyLines
// =============================================================================
describe("parseVocabularyLines", () => {
  it("parses simple term: meaning lines", () => {
    expect(parseVocabularyLines("foo: bar")).toEqual([{ term: "foo", meaning: "bar" }]);
  });

  it("parses multiple lines", () => {
    expect(parseVocabularyLines("foo: bar\nbaz: qux")).toEqual([
      { term: "foo", meaning: "bar" },
      { term: "baz", meaning: "qux" },
    ]);
  });

  it("handles extra whitespace", () => {
    expect(parseVocabularyLines("  foo  :  bar  \n  baz  :  qux  ")).toEqual([
      { term: "foo", meaning: "bar" },
      { term: "baz", meaning: "qux" },
    ]);
  });

  it("skips empty lines", () => {
    expect(parseVocabularyLines("foo: bar\n\n\nbaz: qux")).toEqual([
      { term: "foo", meaning: "bar" },
      { term: "baz", meaning: "qux" },
    ]);
  });

  it("skips lines without colon", () => {
    expect(parseVocabularyLines("foo: bar\nno colon here\nbaz: qux")).toEqual([
      { term: "foo", meaning: "bar" },
      { term: "baz", meaning: "qux" },
    ]);
  });

  it("skips lines with empty term", () => {
    expect(parseVocabularyLines(": no term")).toEqual([]);
  });

  it("skips lines with empty meaning", () => {
    expect(parseVocabularyLines("term:")).toEqual([]);
  });

  it("handles meaning with colons (only splits on first colon)", () => {
    expect(parseVocabularyLines("URL: https://example.com")).toEqual([
      { term: "URL", meaning: "https://example.com" },
    ]);
  });

  it("handles CJK terms and meanings", () => {
    expect(parseVocabularyLines("createReducer: 创建 reducer 的函数，通常用于 Redux Toolkit 中")).toEqual([
      { term: "createReducer", meaning: "创建 reducer 的函数，通常用于 Redux Toolkit 中" },
    ]);
  });

  it("handles meaning with parentheses", () => {
    expect(parseVocabularyLines("mutate: 修改（在这里指直接修改对象）")).toEqual([
      { term: "mutate", meaning: "修改（在这里指直接修改对象）" },
    ]);
  });

  it("handles meaning with markdown", () => {
    expect(parseVocabularyLines("bold: Use **bold** for emphasis")).toEqual([
      { term: "bold", meaning: "Use **bold** for emphasis" },
    ]);
  });

  it("handles meaning with backticks", () => {
    expect(parseVocabularyLines("log: Use `console.log()`")).toEqual([
      { term: "log", meaning: "Use `console.log()`" },
    ]);
  });

  it("returns empty array for empty string", () => {
    expect(parseVocabularyLines("")).toEqual([]);
  });

  it("returns empty array for whitespace-only string", () => {
    expect(parseVocabularyLines("   \n  \n  ")).toEqual([]);
  });

  it("handles Windows line endings", () => {
    expect(parseVocabularyLines("foo: bar\r\nbaz: qux")).toEqual([
      { term: "foo", meaning: "bar" },
      { term: "baz", meaning: "qux" },
    ]);
  });
});

// =============================================================================
// parseReadModeSeparator
// =============================================================================
describe("parseReadModeSeparator", () => {
  // --- Translation only (no separators) ---
  it("returns simple layout for plain translation", () => {
    const result = parseReadModeSeparator("Hello world, this is a translation.");
    expect(result.readLayout).toBe("simple");
    expect(result.readTranslation).toBe("Hello world, this is a translation.");
    expect(result.readSummary).toBe("");
    expect(result.readVocabulary).toEqual([]);
  });

  it("handles multiline translation without separators", () => {
    const text = "Line 1\n\nLine 2\n\nLine 3";
    const result = parseReadModeSeparator(text);
    expect(result.readLayout).toBe("simple");
    expect(result.readTranslation).toBe(text);
  });

  it("handles translation with markdown formatting", () => {
    const text = "This is **bold** and *italic* and `code`\n\n- List item 1\n- List item 2";
    const result = parseReadModeSeparator(text);
    expect(result.readLayout).toBe("simple");
    expect(result.readTranslation).toBe(text);
  });

  it("handles empty string", () => {
    const result = parseReadModeSeparator("");
    expect(result.readLayout).toBe("simple");
    expect(result.readTranslation).toBe("");
  });

  // --- Translation + Vocabulary ---
  it("parses translation with vocabulary", () => {
    const text = "这个对象将被传递给 createReducer。\n---VOCABULARY---\ncreateReducer: 创建 reducer 的函数\nmutate: 修改、改变";
    const result = parseReadModeSeparator(text);
    expect(result.readLayout).toBe("withVocab");
    expect(result.readTranslation).toBe("这个对象将被传递给 createReducer。");
    expect(result.readVocabulary).toEqual([
      { term: "createReducer", meaning: "创建 reducer 的函数" },
      { term: "mutate", meaning: "修改、改变" },
    ]);
    expect(result.readSummary).toBe("");
  });

  it("handles vocabulary with extra whitespace around separator", () => {
    const text = "Translation here.\n\n---VOCABULARY---\n\nfoo: bar\nbaz: qux";
    const result = parseReadModeSeparator(text);
    expect(result.readLayout).toBe("withVocab");
    expect(result.readTranslation).toBe("Translation here.");
    expect(result.readVocabulary).toEqual([
      { term: "foo", meaning: "bar" },
      { term: "baz", meaning: "qux" },
    ]);
  });

  // --- Translation + Summary ---
  it("parses translation with summary", () => {
    const text = "Full translation of a long passage goes here.\n---SUMMARY---\nKey points: this and that.";
    const result = parseReadModeSeparator(text);
    expect(result.readLayout).toBe("withSummary");
    expect(result.readTranslation).toBe("Full translation of a long passage goes here.");
    expect(result.readSummary).toBe("Key points: this and that.");
    expect(result.readVocabulary).toEqual([]);
  });

  it("handles multiline summary", () => {
    const text = "Translation.\n---SUMMARY---\nPoint 1.\nPoint 2.\nPoint 3.";
    const result = parseReadModeSeparator(text);
    expect(result.readLayout).toBe("withSummary");
    expect(result.readSummary).toBe("Point 1.\nPoint 2.\nPoint 3.");
  });

  // --- Translation + Vocabulary + Summary ---
  it("parses all three sections", () => {
    const text = "Translation text.\n---VOCABULARY---\nfoo: bar\nbaz: qux\n---SUMMARY---\nBrief summary.";
    const result = parseReadModeSeparator(text);
    expect(result.readLayout).toBe("withSummary");
    expect(result.readTranslation).toBe("Translation text.");
    expect(result.readVocabulary).toEqual([
      { term: "foo", meaning: "bar" },
      { term: "baz", meaning: "qux" },
    ]);
    expect(result.readSummary).toBe("Brief summary.");
  });

  it("handles all sections with extra whitespace", () => {
    const text = "Translation.\n\n---VOCABULARY---\n\nfoo: bar\n\n---SUMMARY---\n\nSummary text.";
    const result = parseReadModeSeparator(text);
    expect(result.readTranslation).toBe("Translation.");
    expect(result.readVocabulary).toEqual([{ term: "foo", meaning: "bar" }]);
    expect(result.readSummary).toBe("Summary text.");
  });

  // --- Streaming (partial text) ---
  it("handles streaming: only translation so far", () => {
    const text = "这个对象将被传递给 cre";
    const result = parseReadModeSeparator(text);
    expect(result.readLayout).toBe("simple");
    expect(result.readTranslation).toBe(text);
  });

  it("handles streaming: translation complete, vocabulary starting", () => {
    const text = "Translation text.\n---VOCABULARY---\ncreateReducer: 创建 reduce";
    const result = parseReadModeSeparator(text);
    expect(result.readTranslation).toBe("Translation text.");
    // Partial vocabulary line may or may not parse (depends on colon presence)
  });

  it("handles streaming: vocabulary complete, summary starting", () => {
    const text = "Translation.\n---VOCABULARY---\nfoo: bar\n---SUMMARY---\nBrief summ";
    const result = parseReadModeSeparator(text);
    expect(result.readTranslation).toBe("Translation.");
    expect(result.readVocabulary).toEqual([{ term: "foo", meaning: "bar" }]);
    expect(result.readSummary).toBe("Brief summ");
  });

  it("handles streaming: separator partially typed", () => {
    // The --- is typed but not complete yet
    const text = "Translation.\n---VOCAB";
    const result = parseReadModeSeparator(text);
    // No separator match, entire text is translation
    expect(result.readLayout).toBe("simple");
    expect(result.readTranslation).toBe("Translation.\n---VOCAB");
  });

  // --- Edge cases ---
  it("handles translation containing triple dashes (not a separator)", () => {
    const text = "Use --- for horizontal rule in markdown.\nAnother line.";
    const result = parseReadModeSeparator(text);
    expect(result.readLayout).toBe("simple");
    expect(result.readTranslation).toBe(text);
  });

  it("handles translation with markdown code block containing separator-like text", () => {
    const text = "Here is how to use it:\n```\n---VOCABULARY---\nfoo: bar\n```\nEnd.";
    const result = parseReadModeSeparator(text);
    // The separator inside code block will be detected — this is acceptable
    // because LLM output should not contain the separator in content
    expect(result.readTranslation).toBe("Here is how to use it:\n```");
  });

  it("handles vocabulary with special characters in terms", () => {
    const text = "Translation.\n---VOCABULARY---\n`useState`: React 的状态钩子\nArray.map(): 数组映射方法";
    const result = parseReadModeSeparator(text);
    expect(result.readVocabulary).toEqual([
      { term: "`useState`", meaning: "React 的状态钩子" },
      { term: "Array.map()", meaning: "数组映射方法" },
    ]);
  });

  it("handles CJK translation with vocabulary and summary", () => {
    const text = `对象中的键将用于生成字符串操作类型常量，这些常量在分派时会显示在 Redux DevTools Extension 中。

---VOCABULARY---
createReducer: 创建 reducer 的函数，通常用于 Redux Toolkit 中
mutate: 改变、修改（在这里指直接修改状态对象）
state: 状态，指应用程序中由 reducer 管理的数据

---SUMMARY---
对象的键生成 Redux 操作类型常量，会在 DevTools 中显示。同名操作会触发对应 reducer 运行。`;
    const result = parseReadModeSeparator(text);
    expect(result.readLayout).toBe("withSummary");
    expect(result.readTranslation).toContain("对象中的键将用于生成字符串操作类型常量");
    expect(result.readTranslation).not.toContain("---VOCABULARY---");
    expect(result.readTranslation).not.toContain("---SUMMARY---");
    expect(result.readVocabulary.length).toBe(3);
    expect(result.readVocabulary[0].term).toBe("createReducer");
    expect(result.readVocabulary[1].term).toBe("mutate");
    expect(result.readVocabulary[2].term).toBe("state");
    expect(result.readSummary).toContain("对象的键生成 Redux 操作类型常量");
  });
});
