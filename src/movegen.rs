//! 指し手生成・終局判定・perft。

use crate::attacks::*;
use crate::board::*;
use crate::rng::Rng;
use std::mem::MaybeUninit;

/// ヒープ確保なしの固定長リスト（チェスの合法手は最大 218）。
/// 作るたびに 256 手分を初期化しないよう、未初期化領域を使う。
/// 不変条件: `moves[..len]` は初期化済み。
pub struct MoveList {
    moves: [MaybeUninit<Move>; 256],
    len: usize,
}

impl MoveList {
    #[inline]
    pub fn new() -> MoveList {
        MoveList { moves: [const { MaybeUninit::uninit() }; 256], len: 0 }
    }
    #[inline]
    pub fn push(&mut self, m: Move) {
        self.moves[self.len].write(m);
        self.len += 1;
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    #[inline]
    pub fn as_slice(&self) -> &[Move] {
        // SAFETY: 不変条件により先頭 len 個は初期化済み。MaybeUninit<Move> と Move は同じレイアウト。
        unsafe { std::slice::from_raw_parts(self.moves.as_ptr() as *const Move, self.len) }
    }
    /// i 番目を取り除いて返す（順序は保たない）
    #[inline]
    pub fn swap_remove(&mut self, i: usize) -> Move {
        let m = self.as_slice()[i];
        self.len -= 1;
        self.moves[i] = self.moves[self.len];
        m
    }
}

impl Default for MoveList {
    fn default() -> Self {
        Self::new()
    }
}

/// `Position::legality_info` の結果。
pub struct LegalityInfo {
    pub king: u8,
    pub in_check: bool,
    /// 動くと自キングが素通しになりうる自駒（ここに含まれる駒は確実な方法で判定する）
    pub pinned: Bitboard,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameResult {
    WhiteWins,
    BlackWins,
    Draw,
}

impl GameResult {
    /// 白視点のスコア（勝ち 1 / 引き分け 0.5 / 負け 0）
    pub fn white_score(self) -> f64 {
        match self {
            GameResult::WhiteWins => 1.0,
            GameResult::Draw => 0.5,
            GameResult::BlackWins => 0.0,
        }
    }
}

const PROMOS: [Piece; 4] = [Piece::Queen, Piece::Rook, Piece::Bishop, Piece::Knight];

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupKind {
    /// 移動元が 1 つの駒（ナイト・ビショップ・ルーク・クイーン・キング）
    Piece,
    /// ポーン。移動元 = 行き先 - オフセット
    Pawn,
    /// 成るポーン。行き先 1 マスにつき 4 手
    PawnPromo,
}

/// 同じ規則で行き先が決まる手のまとまり（行き先はビットボードで持つ）。
#[derive(Clone, Copy)]
struct Group {
    targets: Bitboard,
    /// Piece なら移動元マス、Pawn / PawnPromo なら 行き先 - 移動元
    from_or_offset: i8,
    kind: GroupKind,
}

impl Group {
    #[inline]
    fn weight(&self) -> usize {
        let n = self.targets.count_ones() as usize;
        if self.kind == GroupKind::PawnPromo { n * 4 } else { n }
    }

    #[inline]
    fn make(&self, to: u8, promo: Option<Piece>) -> Move {
        let from = match self.kind {
            GroupKind::Piece => self.from_or_offset as u8,
            _ => (to as i8 - self.from_or_offset) as u8,
        };
        Move::new(from, to, promo)
    }
}

/// 疑似合法手を、展開せずに「グループの集まり」として持つ。
/// グループ数は最大で ポーン 8 + 駒 16 + キャスリング 2 = 26。
pub struct MoveGroups {
    groups: [MaybeUninit<Group>; 32],
    len: usize,
}

impl MoveGroups {
    #[inline]
    fn new() -> MoveGroups {
        MoveGroups { groups: [const { MaybeUninit::uninit() }; 32], len: 0 }
    }

    #[inline]
    fn push(&mut self, targets: Bitboard, from_or_offset: i8, kind: GroupKind) {
        if targets != 0 {
            self.groups[self.len].write(Group { targets, from_or_offset, kind });
            self.len += 1;
        }
    }

    /// 成るポーンとそれ以外を別グループに分けて追加する。
    #[inline]
    fn push_pawns(&mut self, targets: Bitboard, offset: i8, promo_rank: Bitboard) {
        self.push(targets & !promo_rank, offset, GroupKind::Pawn);
        self.push(targets & promo_rank, offset, GroupKind::PawnPromo);
    }

    #[inline]
    fn as_slice(&self) -> &[Group] {
        // SAFETY: 先頭 len 個は push で初期化済み。
        unsafe { std::slice::from_raw_parts(self.groups.as_ptr() as *const Group, self.len) }
    }

    /// 疑似合法手の総数
    #[inline]
    pub fn count(&self) -> usize {
        self.as_slice().iter().map(Group::weight).sum()
    }

    /// n 番目（0 始まり、n < count()）の疑似合法手を、リストを作らずに取り出す。
    #[inline]
    pub fn nth(&self, mut n: usize) -> Move {
        for g in self.as_slice() {
            let w = g.weight();
            if n < w {
                let (idx, promo) = if g.kind == GroupKind::PawnPromo {
                    (n / 4, Some(PROMOS[n % 4]))
                } else {
                    (n, None)
                };
                let mut bb = g.targets;
                for _ in 0..idx {
                    bb &= bb - 1;
                }
                return g.make(bb.trailing_zeros() as u8, promo);
            }
            n -= w;
        }
        panic!("nth: 範囲外");
    }

    pub fn to_list(&self) -> MoveList {
        let mut list = MoveList::new();
        for g in self.as_slice() {
            for to in Squares(g.targets) {
                if g.kind == GroupKind::PawnPromo {
                    for p in PROMOS {
                        list.push(g.make(to, Some(p)));
                    }
                } else {
                    list.push(g.make(to, None));
                }
            }
        }
        list
    }
}

impl Position {
    /// 疑似合法手（自キングを王手に晒す手も含む）をグループの形で生成する。
    pub fn move_groups(&self) -> MoveGroups {
        let mut g = MoveGroups::new();
        let us = self.side;
        let own = self.colors[us.idx()];
        let enemy = self.colors[us.opp().idx()];
        let occ = own | enemy;
        let empty = !occ;
        let not_own = !own;

        // ポーン（集合演算でまとめて動かす）
        let pawns = self.bb(us, Piece::Pawn);
        let capturable = enemy | self.ep.map_or(0, |s| 1u64 << s);
        if us == Color::White {
            let single = (pawns << 8) & empty;
            let double = ((single & RANK_3) << 8) & empty;
            g.push_pawns(single, 8, RANK_8);
            g.push(double, 16, GroupKind::Pawn);
            g.push_pawns(((pawns & !FILE_A) << 7) & capturable, 7, RANK_8);
            g.push_pawns(((pawns & !FILE_H) << 9) & capturable, 9, RANK_8);
        } else {
            let single = (pawns >> 8) & empty;
            let double = ((single & RANK_6) >> 8) & empty;
            g.push_pawns(single, -8, RANK_1);
            g.push(double, -16, GroupKind::Pawn);
            g.push_pawns(((pawns & !FILE_A) >> 9) & capturable, -9, RANK_1);
            g.push_pawns(((pawns & !FILE_H) >> 7) & capturable, -7, RANK_1);
        }

        for from in Squares(self.bb(us, Piece::Knight)) {
            g.push(KNIGHT[from as usize] & not_own, from as i8, GroupKind::Piece);
        }
        let queens = self.bb(us, Piece::Queen);
        let bishops = self.bb(us, Piece::Bishop);
        let rooks = self.bb(us, Piece::Rook);
        for from in Squares(bishops | rooks | queens) {
            let bit = 1u64 << from;
            let mut t = 0;
            if bit & (bishops | queens) != 0 {
                t |= bishop_attacks(from as usize, occ);
            }
            if bit & (rooks | queens) != 0 {
                t |= rook_attacks(from as usize, occ);
            }
            g.push(t & not_own, from as i8, GroupKind::Piece);
        }
        let king = self.king_sq(us);
        g.push(KING[king as usize] & not_own, king as i8, GroupKind::Piece);
        g.push(self.castling_targets(king, occ), king as i8, GroupKind::Piece);
        g
    }

    /// 自キングを王手に晒す手も含む「疑似合法手」をリストで生成する。
    pub fn pseudo_legal_moves(&self) -> MoveList {
        self.move_groups().to_list()
    }

    /// キャスリングでキングが行けるマス（g1/c1 など）のビットボード。
    fn castling_targets(&self, king: u8, occ: Bitboard) -> Bitboard {
        let (home, k_bit, q_bit) = match self.side {
            Color::White => (4u8, WK, WQ),
            Color::Black => (60u8, BK, BQ),
        };
        if king != home || self.castling & (k_bit | q_bit) == 0 {
            return 0;
        }
        let them = self.side.opp();
        if self.is_attacked(home, them) {
            return 0;
        }
        let mut t = 0;
        // キング側: f, g が空き / クイーン側: b, c, d が空き。キングが通るマスに利きがないこと。
        let k_empty = (1u64 << (home + 1)) | (1u64 << (home + 2));
        let q_empty = (1u64 << (home - 1)) | (1u64 << (home - 2)) | (1u64 << (home - 3));
        if self.castling & k_bit != 0
            && occ & k_empty == 0
            && !self.is_attacked(home + 1, them)
            && !self.is_attacked(home + 2, them)
        {
            t |= 1u64 << (home + 2);
        }
        if self.castling & q_bit != 0
            && occ & q_empty == 0
            && !self.is_attacked(home - 1, them)
            && !self.is_attacked(home - 2, them)
        {
            t |= 1u64 << (home - 2);
        }
        t
    }

    /// 指した後に自キングが攻撃されていないか（局面を作って確かめる確実な方法）。
    pub fn is_legal(&self, m: Move) -> bool {
        let next = self.make_move(m);
        !next.is_attacked(next.king_sq(self.side), self.side.opp())
    }

    /// 合法判定の前計算（王手の有無とピンされた自駒）。
    pub fn legality_info(&self) -> LegalityInfo {
        let us = self.side;
        let them = us.opp();
        let king = self.king_sq(us);
        let k = king as usize;
        let occ = self.occupied();
        let enemy = self.colors[them.idx()];
        let queens = self.bb(them, Piece::Queen);

        // 自駒を素通りしてキングに利く敵の飛び駒（ピンの候補）
        let rook_snipers = rook_attacks(k, enemy) & (self.bb(them, Piece::Rook) | queens);
        let bishop_snipers = bishop_attacks(k, enemy) & (self.bb(them, Piece::Bishop) | queens);
        let mut pinned = 0;
        for s in Squares(rook_snipers) {
            let between = rook_attacks(k, 1 << s) & rook_attacks(s as usize, 1 << king) & occ;
            if between.count_ones() == 1 {
                pinned |= between & self.colors[us.idx()];
            }
        }
        for s in Squares(bishop_snipers) {
            let between = bishop_attacks(k, 1 << s) & bishop_attacks(s as usize, 1 << king) & occ;
            if between.count_ones() == 1 {
                pinned |= between & self.colors[us.idx()];
            }
        }
        LegalityInfo { king, in_check: self.is_attacked(king, them), pinned }
    }

    /// `legality_info` の軽量版。ピンを正確に求めず、キングと同じ縦横斜めの線上にある
    /// 自駒をすべて「要確認」とする。数手しか判定しないプレイアウトではこちらが速い。
    #[inline]
    pub fn legality_info_light(&self) -> LegalityInfo {
        let king = self.king_sq(self.side);
        let k = king as usize;
        let lines = rook_attacks(k, 0) | bishop_attacks(k, 0);
        LegalityInfo {
            king,
            in_check: self.is_attacked(king, self.side.opp()),
            pinned: lines & self.colors[self.side.idx()],
        }
    }

    /// `is_legal` と同じ判定を、多くの手で局面のコピーなしに行う。
    #[inline]
    pub fn is_legal_with(&self, m: Move, info: &LegalityInfo) -> bool {
        if m.from == info.king {
            // キングの移動: キングを抜いた盤で行き先に利きがないか
            // （キャスリングの通過マスは生成時に確認済み）
            let occ = self.occupied() ^ (1u64 << info.king);
            return !self.is_attacked_with_occ(m.to, self.side.opp(), occ);
        }
        let is_ep = Some(m.to) == self.ep && self.bb(self.side, Piece::Pawn) & (1u64 << m.from) != 0;
        if info.in_check || is_ep || info.pinned & (1u64 << m.from) != 0 {
            // 王手中・アンパッサン・ピンされた駒は頻度が低いので確実な方法で
            return self.is_legal(m);
        }
        true
    }

    pub fn legal_moves(&self) -> MoveList {
        let pseudo = self.pseudo_legal_moves();
        let info = self.legality_info();
        let mut list = MoveList::new();
        for &m in pseudo.as_slice() {
            if self.is_legal_with(m, &info) {
                list.push(m);
            }
        }
        list
    }

    /// 合法手を 1 つ一様ランダムに選ぶ。無ければ None。
    ///
    /// 疑似合法手から一様に引き、非合法なら引き直す（棄却サンプリング）。
    /// 引き直しても分布は合法手上の一様分布のまま。手のリストを作らず、
    /// 引いた 1 手だけを組み立てて判定するのでプレイアウトが速い。
    pub fn random_legal_move(&self, rng: &mut Rng) -> Option<Move> {
        let groups = self.move_groups();
        let total = groups.count();
        if total == 0 {
            return None;
        }
        let info = self.legality_info_light();
        // 王手中は非合法手が多いので、最初からリスト方式にする
        if !info.in_check {
            for _ in 0..8 {
                let m = groups.nth(rng.below(total));
                if self.is_legal_with(m, &info) {
                    return Some(m);
                }
            }
        }
        // 引き直しが続いた（合法手が少ない・無い）ときは全列挙して選ぶ
        let mut list = groups.to_list();
        while !list.is_empty() {
            let i = rng.below(list.len());
            let m = list.as_slice()[i];
            if self.is_legal_with(m, &info) {
                return Some(m);
            }
            list.swap_remove(i);
        }
        None
    }

    /// 駒が足りず詰みが不可能か（K vs K、K+軽駒 vs K、同色マスのビショップのみ）
    pub fn insufficient_material(&self) -> bool {
        let p = &self.pieces;
        if p[Piece::Pawn as usize] | p[Piece::Rook as usize] | p[Piece::Queen as usize] != 0 {
            return false;
        }
        let knights = p[Piece::Knight as usize];
        let bishops = p[Piece::Bishop as usize];
        (knights | bishops).count_ones() <= 1
            || (knights == 0 && (bishops & DARK_SQUARES == 0 || bishops & !DARK_SQUARES == 0))
    }

    /// 終局していれば結果を返す（千日手は履歴が要るので含まない）。
    pub fn result(&self) -> Option<GameResult> {
        let info = self.legality_info();
        let has_move = self
            .pseudo_legal_moves()
            .as_slice()
            .iter()
            .any(|&m| self.is_legal_with(m, &info));
        if !has_move {
            return Some(if self.in_check() {
                match self.side {
                    Color::White => GameResult::BlackWins,
                    Color::Black => GameResult::WhiteWins,
                }
            } else {
                GameResult::Draw
            });
        }
        if self.halfmove >= 100 || self.insufficient_material() {
            return Some(GameResult::Draw);
        }
        None
    }

    pub fn perft(&self, depth: u32) -> u64 {
        if depth == 0 {
            return 1;
        }
        let moves = self.legal_moves();
        if depth == 1 {
            return moves.len() as u64;
        }
        moves
            .as_slice()
            .iter()
            .map(|&m| self.make_move(m).perft(depth - 1))
            .sum()
    }

    /// UCI 文字列を合法手に変換する。
    pub fn parse_move(&self, s: &str) -> Option<Move> {
        self.legal_moves()
            .as_slice()
            .iter()
            .copied()
            .find(|m| m.to_uci() == s.trim().to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(fen: &str, expected: &[u64]) {
        let pos = Position::from_fen(fen).unwrap();
        for (i, &n) in expected.iter().enumerate() {
            assert_eq!(pos.perft(i as u32 + 1), n, "{fen} depth {}", i + 1);
        }
    }

    #[test]
    fn perft_startpos() {
        check(START_FEN, &[20, 400, 8902, 197281]);
    }

    #[test]
    fn perft_kiwipete() {
        check(
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            &[48, 2039, 97862],
        );
    }

    #[test]
    fn perft_position3() {
        check("8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1", &[14, 191, 2812, 43238]);
    }

    #[test]
    fn perft_position4() {
        check(
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            &[6, 264, 9467],
        );
    }

    #[test]
    fn perft_position5() {
        check("rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8", &[44, 1486, 62379]);
    }

    /// 高速な合法判定が、局面を作って確かめる方法と常に一致するか。
    /// ランダム対局で現れる多数の局面で照合する。
    #[test]
    fn fast_legality_matches_slow() {
        let mut rng = Rng::new(7);
        for _ in 0..300 {
            let mut pos = Position::startpos();
            for _ in 0..200 {
                let info = pos.legality_info();
                let light = pos.legality_info_light();
                for &m in pos.pseudo_legal_moves().as_slice() {
                    let slow = pos.is_legal(m);
                    assert_eq!(pos.is_legal_with(m, &info), slow, "{} {m}", pos.to_fen());
                    assert_eq!(pos.is_legal_with(m, &light), slow, "light {} {m}", pos.to_fen());
                }
                match pos.random_legal_move(&mut rng) {
                    Some(m) => pos = pos.make_move(m),
                    None => break,
                }
            }
        }
    }

    /// random_legal_move が合法手を偏りなく選ぶか（成り・王手中の局面も含む）。
    #[test]
    fn random_move_is_uniform() {
        let fens = [
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
        ];
        let mut rng = Rng::new(1);
        for fen in fens {
            let pos = Position::from_fen(fen).unwrap();
            let legal = pos.legal_moves();
            let n = legal.len();
            let per_move = 4000;
            let mut counts = vec![0u32; n];
            for _ in 0..n * per_move {
                let m = pos.random_legal_move(&mut rng).unwrap();
                let i = legal.as_slice().iter().position(|&x| x == m).expect("非合法手");
                counts[i] += 1;
            }
            for (i, &c) in counts.iter().enumerate() {
                // 期待値 4000、標準偏差は約 63。±8σ を許容。
                assert!((c as i64 - per_move as i64).abs() < 500, "{fen}: {} が {c} 回", legal.as_slice()[i]);
            }
        }
    }

    #[test]
    fn fen_roundtrip() {
        let fen = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
        assert_eq!(Position::from_fen(fen).unwrap().to_fen(), fen);
    }

    #[test]
    fn mate_and_stalemate() {
        // フールズメイト
        let mate = Position::from_fen("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3").unwrap();
        assert_eq!(mate.result(), Some(GameResult::BlackWins));
        // ステイルメイト
        let stale = Position::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").unwrap();
        assert_eq!(stale.result(), Some(GameResult::Draw));
    }
}
