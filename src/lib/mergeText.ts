/// 行単位の 3-way マージ (diff3 相当)。クエリファイルが外部で変更された時に、
/// 手元の未保存編集 (local) と外部の変更 (remote) を共通の元 (base) を基準に
/// 突き合わせ、変更が別々の行に及んでいれば自動マージし、同じ行を双方が
/// 別々に変更した場合のみ conflict=true として返す。
///
/// 純粋関数 (Tauri 非依存) なので単体で検証できる。SQL ファイルは小さいため
/// LCS は素朴な O(n*m) DP で十分。

export interface Merge3Result {
  /// マージ結果テキスト。conflict=true の時は「片側を選んだだけ」の中途半端な
  /// 内容になり得るので、呼び出し側は使わないこと。
  merged: string;
  /// 同じ領域を local と remote が別々に変更したため自動マージできなかった。
  conflict: boolean;
}

/// LCS が O(n*m) のため、この行数を超えるファイルはマージを試みず衝突扱いにする
/// (メインスレッドの長時間停止・巨大な DP 配列によるメモリ圧迫を避ける)。
/// これを超えるのは通常のクエリファイルでは考えにくく、超えた場合は手動解決に委ねる。
const MAX_MERGE_LINES = 20000;

/// text を行配列へ分解する。join("\n") で元に戻せる可逆な分割にするため
/// split("\n") を使う ("a\nb" -> ["a","b"], "a\nb\n" -> ["a","b",""]).
function splitLines(text: string): string[] {
  return text.split("\n");
}

/// base と other の最長共通部分列に含まれる添字ペア (増加順) を返す。
function lcsPairs(base: string[], other: string[]): Array<[number, number]> {
  const n = base.length;
  const m = other.length;
  // dp[i][j] = base[i:] と other[j:] の LCS 長
  const dp: number[][] = Array.from({ length: n + 1 }, () =>
    new Array<number>(m + 1).fill(0),
  );
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] =
        base[i] === other[j]
          ? dp[i + 1][j + 1] + 1
          : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const pairs: Array<[number, number]> = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (base[i] === other[j]) {
      pairs.push([i, j]);
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      i++;
    } else {
      j++;
    }
  }
  return pairs;
}

interface DiffRegion {
  /// base 側で変更された範囲 [oStart, oStart+oLen)
  oStart: number;
  oLen: number;
  /// other 側の対応する範囲 [tStart, tStart+tLen)
  tStart: number;
  tLen: number;
}

/// base から other への変更領域 (共通部分に挟まれた差分ブロック) を返す。
function diffRegions(base: string[], other: string[]): DiffRegion[] {
  const pairs = lcsPairs(base, other);
  const regions: DiffRegion[] = [];
  let oi = 0;
  let ti = 0;
  for (const [bi, tj] of pairs) {
    if (bi > oi || tj > ti) {
      regions.push({ oStart: oi, oLen: bi - oi, tStart: ti, tLen: tj - ti });
    }
    oi = bi + 1;
    ti = tj + 1;
  }
  if (base.length > oi || other.length > ti) {
    regions.push({
      oStart: oi,
      oLen: base.length - oi,
      tStart: ti,
      tLen: other.length - ti,
    });
  }
  return regions;
}

interface Hunk {
  oStart: number;
  oLen: number;
  /// 0 = local(A), 2 = remote(B) (diff3 の慣習に合わせる)
  side: 0 | 2;
  sideStart: number;
  sideLen: number;
}

/// base を共通の元として local と remote を 3-way マージする。
export function merge3(
  baseText: string,
  localText: string,
  remoteText: string,
): Merge3Result {
  const base = splitLines(baseText);
  const local = splitLines(localText);
  const remote = splitLines(remoteText);

  // 巨大なファイルは自動マージを諦め衝突扱いにする (呼び出し側が警告する)。
  if (
    base.length > MAX_MERGE_LINES ||
    local.length > MAX_MERGE_LINES ||
    remote.length > MAX_MERGE_LINES
  ) {
    return { merged: localText, conflict: true };
  }

  // base に対する両側の変更領域を hunk として集め、base 座標順に並べる。
  const hunks: Hunk[] = [];
  for (const r of diffRegions(base, local)) {
    hunks.push({
      oStart: r.oStart,
      oLen: r.oLen,
      side: 0,
      sideStart: r.tStart,
      sideLen: r.tLen,
    });
  }
  for (const r of diffRegions(base, remote)) {
    hunks.push({
      oStart: r.oStart,
      oLen: r.oLen,
      side: 2,
      sideStart: r.tStart,
      sideLen: r.tLen,
    });
  }
  hunks.sort((x, y) => x.oStart - y.oStart || x.side - y.side);

  const out: string[] = [];
  let conflict = false;
  let cursor = 0; // 未出力の base 位置

  let k = 0;
  while (k < hunks.length) {
    // base 座標で重なり合う hunk 群を 1 つの領域にまとめる。端点で接するだけ
    // (隣接する別々の行への変更) は重なりとみなさず別領域として扱う (< で判定)。
    // ただし同じ点への挿入 (oLen=0 同士が同じ oStart) は < では捕まらないので、
    // 開始位置が同一の hunk も同一領域に含める (双方が同じ箇所へ別内容を挿入した
    // ケースを衝突として検出するため)。
    const regionStart = hunks[k].oStart;
    let regionEnd = hunks[k].oStart + hunks[k].oLen;
    const group: Hunk[] = [hunks[k]];
    k++;
    while (
      k < hunks.length &&
      (hunks[k].oStart < regionEnd || hunks[k].oStart === regionStart)
    ) {
      regionEnd = Math.max(regionEnd, hunks[k].oStart + hunks[k].oLen);
      group.push(hunks[k]);
      k++;
    }

    // 領域より前の未変更 base をそのまま出力する。
    if (regionStart > cursor) {
      for (let i = cursor; i < regionStart; i++) out.push(base[i]);
    }

    // 各側の、領域 [regionStart, regionEnd) に対応する内容を復元する。
    // 側の hunk が無ければ base のまま。hunk があれば、変更ブロックの前後の
    // 一致行は base と 1:1 対応する性質を使い、side 座標へ換算する。
    const sideContent = (side: 0 | 2, src: string[]): string[] => {
      const parts = group.filter((h) => h.side === side);
      if (parts.length === 0) {
        return base.slice(regionStart, regionEnd);
      }
      let oMin = Infinity;
      let oMax = -Infinity;
      let sMin = Infinity;
      let sMax = -Infinity;
      for (const h of parts) {
        oMin = Math.min(oMin, h.oStart);
        oMax = Math.max(oMax, h.oStart + h.oLen);
        sMin = Math.min(sMin, h.sideStart);
        sMax = Math.max(sMax, h.sideStart + h.sideLen);
      }
      const lead = oMin - regionStart; // 領域先頭〜変更開始の一致行数
      const trail = regionEnd - oMax; // 変更終端〜領域末尾の一致行数
      return src.slice(sMin - lead, sMax + trail);
    };

    const hasA = group.some((h) => h.side === 0);
    const hasB = group.some((h) => h.side === 2);
    const aContent = sideContent(0, local);
    const bContent = sideContent(2, remote);

    if (hasA && hasB) {
      if (aContent.join("\n") === bContent.join("\n")) {
        // 双方が同じ変更 → どちらでもよい
        for (const line of aContent) out.push(line);
      } else {
        // 同じ領域を別々に変更 → コンフリクト。呼び出し側は merged を使わない。
        conflict = true;
        for (const line of aContent) out.push(line);
      }
    } else if (hasA) {
      for (const line of aContent) out.push(line);
    } else {
      for (const line of bContent) out.push(line);
    }
    cursor = regionEnd;
  }

  // 残りの未変更 base を出力する。
  for (let i = cursor; i < base.length; i++) out.push(base[i]);

  return { merged: out.join("\n"), conflict };
}
