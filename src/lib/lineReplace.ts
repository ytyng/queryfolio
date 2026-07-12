// 行単位の一括置換。template 内の %%% を入力の各行で置換して 1 行ずつ出力する。
// 空行・# 始まり・// 始まりの行はスキップする (t.ytyng.com/line-replace 互換)。
// 主な用途: SHOW FULL PROCESSLIST の ID 一覧を `KILL %%%;` に一括変換する等。
export const PLACEHOLDER = "%%%";

export const generateLineReplace = (
  lines: string,
  template: string,
): string => {
  const out: string[] = [];
  for (const raw of lines.split(/\r?\n/)) {
    const line = raw.trim();
    if (line === "" || line.startsWith("#") || line.startsWith("//")) {
      continue;
    }
    // %%% の全出現を置換する。split/join なので正規表現の特殊文字を気にしない
    out.push(template.split(PLACEHOLDER).join(line));
  }
  return out.join("\n");
};

// 出力行数 (プレビューの「N lines」表示用)。空入力では 0
export const countLineReplaceResults = (
  lines: string,
  template: string,
): number => {
  const result = generateLineReplace(lines, template);
  return result === "" ? 0 : result.split("\n").length;
};
