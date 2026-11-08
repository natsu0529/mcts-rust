//! 利きテーブル（コンパイル時に計算）と飛び駒の利き計算。
//!
//! ビットボード: u64 の bit n がマス n（a1=0, h8=63）に対応する。

pub type Bitboard = u64;

pub const FILE_A: Bitboard = 0x0101_0101_0101_0101;
pub const FILE_H: Bitboard = FILE_A << 7;
pub const RANK_1: Bitboard = 0xFF;
pub const RANK_3: Bitboard = RANK_1 << 16;
pub const RANK_6: Bitboard = RANK_1 << 40;
pub const RANK_8: Bitboard = RANK_1 << 56;
/// 黒マス（a1 は黒マス）
pub const DARK_SQUARES: Bitboard = 0xAA55_AA55_AA55_AA55;

const fn leaper_table(deltas: &[(i8, i8)]) -> [Bitboard; 64] {
    let mut table = [0; 64];
    let mut sq = 0;
    while sq < 64 {
        let (f, r) = ((sq % 8) as i8, (sq / 8) as i8);
        let mut i = 0;
        while i < deltas.len() {
            let (nf, nr) = (f + deltas[i].0, r + deltas[i].1);
            if nf >= 0 && nf < 8 && nr >= 0 && nr < 8 {
                table[sq] |= 1 << (nr * 8 + nf);
            }
            i += 1;
        }
        sq += 1;
    }
    table
}

pub const KNIGHT: [Bitboard; 64] =
    leaper_table(&[(1, 2), (2, 1), (2, -1), (1, -2), (-1, -2), (-2, -1), (-2, 1), (-1, 2)]);
pub const KING: [Bitboard; 64] =
    leaper_table(&[(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)]);
/// PAWN[color][sq]: その色のポーンが sq にいるときの利き
pub const PAWN: [[Bitboard; 64]; 2] = [leaper_table(&[(-1, 1), (1, 1)]), leaper_table(&[(-1, -1), (1, -1)])];

// 8 方向。前半 4 つはマス番号が増える向き（正方向）、後半 4 つは減る向き。
const DIRS: [(i8, i8); 8] = [(0, 1), (1, 1), (1, 0), (-1, 1), (0, -1), (-1, -1), (-1, 0), (1, -1)];
const N: usize = 0;
const NE: usize = 1;
const E: usize = 2;
const NW: usize = 3;
const S: usize = 4;
const SW: usize = 5;
const W: usize = 6;
const SE: usize = 7;

/// RAYS[dir][sq]: sq から dir 方向へ盤端まで伸ばした線（sq 自身は含まない）
const RAYS: [[Bitboard; 64]; 8] = {
    let mut rays = [[0; 64]; 8];
    let mut d = 0;
    while d < 8 {
        let mut sq = 0;
        while sq < 64 {
            let (mut f, mut r) = ((sq % 8) as i8 + DIRS[d].0, (sq / 8) as i8 + DIRS[d].1);
            while f >= 0 && f < 8 && r >= 0 && r < 8 {
                rays[d][sq] |= 1 << (r * 8 + f);
                f += DIRS[d].0;
                r += DIRS[d].1;
            }
            sq += 1;
        }
        d += 1;
    }
    rays
};

/// 最初にぶつかる駒までの線。ぶつかった駒のマスは含む（取れるかは呼び出し側で判定）。
#[inline(always)]
fn ray(dir: usize, sq: usize, occ: Bitboard) -> Bitboard {
    let line = RAYS[dir][sq];
    let blockers = line & occ;
    if blockers == 0 {
        return line;
    }
    let first = if dir < 4 {
        blockers.trailing_zeros()
    } else {
        63 - blockers.leading_zeros()
    };
    line ^ RAYS[dir][first as usize]
}

#[inline]
pub fn rook_attacks(sq: usize, occ: Bitboard) -> Bitboard {
    ray(N, sq, occ) | ray(E, sq, occ) | ray(S, sq, occ) | ray(W, sq, occ)
}

#[inline]
pub fn bishop_attacks(sq: usize, occ: Bitboard) -> Bitboard {
    ray(NE, sq, occ) | ray(NW, sq, occ) | ray(SE, sq, occ) | ray(SW, sq, occ)
}

/// 最下位ビットのマスを取り出して消すイテレータ。
pub struct Squares(pub Bitboard);

impl Iterator for Squares {
    type Item = u8;
    #[inline]
    fn next(&mut self) -> Option<u8> {
        if self.0 == 0 {
            return None;
        }
        let sq = self.0.trailing_zeros() as u8;
        self.0 &= self.0 - 1;
        Some(sq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables() {
        assert_eq!(KNIGHT[0].count_ones(), 2); // a1
        assert_eq!(KNIGHT[27].count_ones(), 8); // d4
        assert_eq!(KING[0].count_ones(), 3);
        assert_eq!(rook_attacks(0, 0).count_ones(), 14);
        assert_eq!(bishop_attacks(27, 0).count_ones(), 13);
        // a1 のルーク、a3 に駒 → a2, a3 と 1 段目 7 マス
        assert_eq!(rook_attacks(0, 1 << 16).count_ones(), 9);
    }
}
