/// 結果グリッドのセル編集を UPDATE 文へ変換するためのヘルパー。
///
/// 安全側に倒した設計 (AGENTS.md / タスクの設計判断に対応):
/// - 編集できるのは「単一テーブルの SELECT」かつ「結果に主キー列が揃っている」時だけ。
///   JOIN / 集計 / サブクエリ / 別名列などは編集不可 (singleTableSelectTable が null)。
/// - WHERE は主キーで組む。主キー列自体は編集不可 (行の同定が壊れるため)。
/// - 値のクオートはエンジン別。曖昧なケースは Edit ボタン (SQL をエディタに貼る) で
///   手直しできるため、ここでは素直な推定に留める。

export type NormalizedEngine = "mysql" | "postgres" | "sqlite";

/// ConnectionInfo.engine の表記ゆれ (mariadb / postgresql / sqlite3 等) を正規化する。
export function normalizeEngine(engine: string): NormalizedEngine {
  const e = engine.toLowerCase();
  if (e === "mysql" || e === "mariadb") return "mysql";
  if (e === "postgres" || e === "postgresql") return "postgres";
  return "sqlite";
}

/// 単純な識別子 (schema.table を含む) の形か。バックエンドの
/// validate_relation_name と同じ規則にそろえる (先頭は英字か _、以降は
/// 英数字 / _ / $、ドット区切りは 2 つまで)。引用符付き識別子は対象外。
function isPlainIdentifier(token: string): boolean {
  const parts = token.split(".");
  if (parts.length === 0 || parts.length > 2) return false;
  return parts.every((p) => /^[A-Za-z_][A-Za-z0-9_$]*$/.test(p));
}

/// SQL からコメント (-- 行, /* */ ブロック) を除去する。文字列リテラルは温存する。
function stripComments(sql: string): string {
  let out = "";
  let i = 0;
  const n = sql.length;
  while (i < n) {
    const c = sql[i];
    const next = sql[i + 1];
    // 文字列リテラル (' または ") はそのまま通す
    if (c === "'" || c === '"' || c === "`") {
      const quote = c;
      out += c;
      i++;
      while (i < n) {
        out += sql[i];
        if (sql[i] === quote) {
          // '' / "" / `` によるエスケープは 1 文字進めて継続
          if (sql[i + 1] === quote) {
            out += sql[i + 1];
            i += 2;
            continue;
          }
          i++;
          break;
        }
        i++;
      }
      continue;
    }
    if (c === "-" && next === "-") {
      while (i < n && sql[i] !== "\n") i++;
      continue;
    }
    if (c === "/" && next === "*") {
      i += 2;
      while (i < n && !(sql[i] === "*" && sql[i + 1] === "/")) i++;
      i += 2;
      out += " ";
      continue;
    }
    out += c;
    i++;
  }
  return out;
}

/// SELECT が「単一テーブルからの取得」なら、そのテーブル名 (単純識別子) を返す。
/// 少しでも判定が怪しい形 (JOIN / カンマ結合 / サブクエリ / UNION / 引用符付き名 /
/// 別名付き) は編集不可として null を返す (安全側)。
export function singleTableSelectTable(sql: string): string | null {
  const cleaned = stripComments(sql).trim();
  if (!cleaned) return null;

  // 文字列リテラル / 引用識別子を潰しつつ、括弧の深さを追う。深さ 0 の位置だけを
  // キーワード探索の対象にする (サブクエリや関数引数の中は無視する)。
  // 併せて、深さ 0 に現れる語をトークン列として集める。
  const lower = cleaned.toLowerCase();
  let depth = 0;
  let inString: string | null = null;
  // 深さ 0 のテキストだけを連結したもの (キーワード / カンマ検出用)。
  let topText = "";
  for (let i = 0; i < cleaned.length; i++) {
    const c = cleaned[i];
    if (inString) {
      if (c === inString) {
        if (cleaned[i + 1] === inString) {
          i++;
          continue;
        }
        inString = null;
      }
      continue;
    }
    if (c === "'" || c === '"' || c === "`") {
      inString = c;
      // 引用の中身は topText に出さない (プレースホルダを置く)
      if (depth === 0) topText += "\0";
      continue;
    }
    if (c === "(") {
      // 深さ 0 の開き括弧はプレースホルダを残す。こうしないと FROM 直後の
      // サブクエリ "(SELECT ...) alias" が中身を消されて別名だけ残り、単一
      // テーブルに誤認される。
      if (depth === 0) topText += "\0";
      depth++;
      continue;
    }
    if (c === ")") {
      if (depth > 0) depth--;
      continue;
    }
    if (depth === 0) topText += lower[i];
  }

  // SELECT で始まること (WITH / EXPLAIN / VALUES などは対象外)
  if (!/^\s*select\b/.test(topText)) return null;
  // 集合演算・JOIN・深さ 0 のカンマ結合があれば単一テーブルではない。
  // GROUP BY / HAVING は行を集約し、表示される PK 列が「グループの代表行」の
  // 任意の値になり得る (WHERE pk = <代表値> が意図しない実行を更新する) ため弾く。
  if (/\b(join|union|intersect|except|group|having)\b/.test(topText)) {
    return null;
  }

  // 深さ 0 の FROM を探す
  const fromMatch = /\bfrom\b/.exec(topText);
  if (!fromMatch) return null;

  // SELECT リストが編集に使える形かを検証する。表示列と実テーブル列の対応を
  // 保証するため、"*" か「別名・式を含まない素の列名の並び」だけを許可する。
  // 例: `SELECT id+1 AS id, b AS a FROM t` は表示値と実列がズレて別行/別列を
  // 更新し得るため弾く (topText は小文字化済み・括弧/文字列は \0 に潰れている)。
  const selectList = topText
    .slice(0, fromMatch.index)
    .replace(/^\s*select\s+/, "")
    .replace(/^(?:distinct|all)\s+/, "")
    .trim();
  if (selectList !== "*") {
    const items = selectList.split(",").map((s) => s.trim());
    if (!items.every((it) => /^[a-z_][a-z0-9_$]*$/.test(it))) return null;
  }

  const afterFrom = topText.slice(fromMatch.index + 4);
  // FROM 以降を次の句キーワード / セミコロンまでで切る
  const clause = afterFrom.split(
    /\b(where|group|having|order|limit|offset|window|for|fetch)\b|;/,
  )[0];
  const ref = clause.trim();
  // 引用符プレースホルダ (\0) を含む・カンマがある・空なら不可
  if (!ref || ref.includes("\0") || ref.includes(",")) return null;
  // 別名付き ("users u" や "users as u") は空白で複数トークンになる → 不可
  const tokens = ref.split(/\s+/).filter(Boolean);
  if (tokens.length !== 1) return null;
  const table = tokens[0];
  if (!isPlainIdentifier(table)) return null;
  // topText は小文字化しているため、元 SQL から実際の表記を取り出す
  return extractOriginalTable(cleaned, table);
}

/// 小文字化した table 名に対応する元 SQL 中の実表記を返す (大文字小文字を保つ)。
function extractOriginalTable(cleaned: string, lowerTable: string): string | null {
  const re = new RegExp(
    `\\bfrom\\s+(${lowerTable.replace(/[.$]/g, "\\$&")})\\b`,
    "i",
  );
  const m = re.exec(cleaned);
  return m ? m[1] : lowerTable;
}

/// 検出した (引用符なし単純) テーブル名を、エンジンの引用符なし識別子の
/// 畳み込み規則に合わせて正規化する。これを引用してから UPDATE に埋め込む。
/// - PostgreSQL: 引用符なし識別子は小文字に畳まれるため小文字化する
///   (`FROM Users` は実テーブル `users`。畳まないと `UPDATE "Users"` が
///   relation does not exist で失敗する)。
/// - MySQL: 引用符なし識別子は畳まれない (大小はストレージ依存) ため保持。
/// - SQLite: 識別子照合が大小無視なので保持で問題ない。
export function normalizeTableName(
  engine: NormalizedEngine,
  table: string,
): string {
  return engine === "postgres" ? table.toLowerCase() : table;
}

/// 識別子をエンジン別にクオートする。
export function quoteIdent(engine: NormalizedEngine, ident: string): string {
  if (engine === "mysql") return "`" + ident.replace(/`/g, "``") + "`";
  return '"' + ident.replace(/"/g, '""') + '"';
}

/// schema.table 形式なら各パートを個別にクオートする。
export function quoteQualified(engine: NormalizedEngine, table: string): string {
  return table
    .split(".")
    .map((p) => quoteIdent(engine, p))
    .join(".");
}

/// 文字列をエンジン別の SQL リテラルにする。
function quoteString(engine: NormalizedEngine, s: string): string {
  if (engine === "mysql") {
    // MySQL は既定でバックスラッシュがエスケープ文字なので二重化する
    return "'" + s.replace(/\\/g, "\\\\").replace(/'/g, "''") + "'";
  }
  return "'" + s.replace(/'/g, "''") + "'";
}

const NUMERIC_RE = /^-?\d+(\.\d+)?$/;

/// 元セルの値 (JSON) を WHERE 用の SQL リテラルにする。
export function literalFromValue(
  engine: NormalizedEngine,
  value: unknown,
): string {
  if (value === null || value === undefined) return "NULL";
  if (typeof value === "number") return String(value);
  if (typeof value === "boolean") {
    if (engine === "postgres") return value ? "TRUE" : "FALSE";
    return value ? "1" : "0";
  }
  // 文字列 (日時・大きな整数の文字列化なども含む)
  return quoteString(engine, String(value));
}

/// ユーザーが入力した新しい値を、元セルの型を手がかりに SQL リテラルへ変換する。
/// 数値列には数値リテラル、真偽列には真偽リテラル、それ以外は文字列にする。
export function literalFromInput(
  engine: NormalizedEngine,
  original: unknown,
  input: string,
): string {
  if (typeof original === "number") {
    return NUMERIC_RE.test(input.trim())
      ? input.trim()
      : quoteString(engine, input);
  }
  if (typeof original === "boolean") {
    const t = input.trim().toLowerCase();
    const truthy = t === "true" || t === "t" || t === "1";
    const falsy = t === "false" || t === "f" || t === "0";
    if (truthy || falsy) {
      if (engine === "postgres") return truthy ? "TRUE" : "FALSE";
      return truthy ? "1" : "0";
    }
    return quoteString(engine, input);
  }
  // 元が null / 文字列: 列の型が分からないので常に文字列リテラルにする。
  // (null セルに "00123" 等を入れた時に数値へ変形するのを防ぐ。数値列へは
  //  文字列リテラルでも代入時キャストが効くので実害は少ない。)
  return quoteString(engine, input);
}

/// 1 セル分の編集。
export interface CellEdit {
  rowIndex: number;
  column: string;
  original: unknown;
  input: string;
}

/// 主キーと編集内容から UPDATE 文の配列を作る (1 行 = 1 文、同一行の複数編集は
/// まとめて 1 文にする)。順序は行番号順で安定させる。
export function buildUpdateStatements(
  engine: NormalizedEngine,
  table: string,
  pkColumns: string[],
  columns: string[],
  rows: unknown[][],
  edits: CellEdit[],
): string[] {
  const pkIndexes = pkColumns.map((pk) => columns.indexOf(pk));
  // 行番号ごとに編集をまとめる
  const byRow = new Map<number, CellEdit[]>();
  for (const e of edits) {
    const list = byRow.get(e.rowIndex) ?? [];
    list.push(e);
    byRow.set(e.rowIndex, list);
  }
  const qualifiedTable = quoteQualified(engine, table);
  const statements: string[] = [];
  for (const rowIndex of [...byRow.keys()].sort((a, b) => a - b)) {
    const rowEdits = byRow.get(rowIndex)!;
    const row = rows[rowIndex];
    const setClause = rowEdits
      .map(
        (e) =>
          `${quoteIdent(engine, e.column)} = ${literalFromInput(engine, e.original, e.input)}`,
      )
      .join(", ");
    const whereClause = pkColumns
      .map((pk, i) => {
        const value = row[pkIndexes[i]];
        const lit = literalFromValue(engine, value);
        return lit === "NULL"
          ? `${quoteIdent(engine, pk)} IS NULL`
          : `${quoteIdent(engine, pk)} = ${lit}`;
      })
      .join(" AND ");
    statements.push(
      `UPDATE ${qualifiedTable} SET ${setClause} WHERE ${whereClause}`,
    );
  }
  return statements;
}
