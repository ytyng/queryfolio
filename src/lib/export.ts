import type { QueryResult } from "$lib/api";

const cellToString = (value: unknown): string => {
  if (value === null || value === undefined) {
    return "";
  }
  if (typeof value === "object") {
    return JSON.stringify(value);
  }
  return String(value);
};

// 表計算ソフトへの貼り付けで数式として解釈される文字。
// DB 由来の文字列値のみエスケープ対象とし、数値型 (-1 等) は壊さない。
const FORMULA_TRIGGER = /^[=+\-@\t\r]/;

const escapeFormulaInjection = (value: unknown, text: string): string => {
  if (typeof value === "string" && FORMULA_TRIGGER.test(text)) {
    return `'${text}`;
  }
  return text;
};

const escapeCsvField = (field: string): string => {
  if (/[",\n\r]/.test(field)) {
    return `"${field.replace(/"/g, '""')}"`;
  }
  return field;
};

export const toCsv = (result: QueryResult): string => {
  const lines = [result.columns.map(escapeCsvField).join(",")];
  for (const row of result.rows) {
    lines.push(
      row
        .map((v) => escapeCsvField(escapeFormulaInjection(v, cellToString(v))))
        .join(","),
    );
  }
  return lines.join("\n");
};

export const toTsv = (result: QueryResult): string => {
  const sanitize = (field: string) =>
    field.replace(/\t/g, " ").replace(/\r?\n/g, " ");
  const lines = [result.columns.map(sanitize).join("\t")];
  for (const row of result.rows) {
    lines.push(
      row
        .map((v) => sanitize(escapeFormulaInjection(v, cellToString(v))))
        .join("\t"),
    );
  }
  return lines.join("\n");
};

// JOIN 等で同名カラムが並んだ場合に後勝ちで値が消えないよう、
// 2 個目以降に _2, _3 ... を付けて一意化する。
const uniqueColumnKeys = (columns: string[]): string[] => {
  const counts = new Map<string, number>();
  return columns.map((column) => {
    const seen = counts.get(column) ?? 0;
    counts.set(column, seen + 1);
    return seen === 0 ? column : `${column}_${seen + 1}`;
  });
};

export const toJson = (result: QueryResult): string => {
  const keys = uniqueColumnKeys(result.columns);
  const records = result.rows.map((row) =>
    Object.fromEntries(keys.map((key, i) => [key, row[i]])),
  );
  return JSON.stringify(records, null, 2);
};
