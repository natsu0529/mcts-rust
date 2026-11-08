//! 盤面表現（ビットボード）・FEN・利き判定・指し手の適用。
//!
//! マス番号は a1=0, b1=1, ..., h1=7, a2=8, ..., h8=63。
//! file = sq & 7, rank = sq >> 3。

use crate::attacks::*;
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Color {
    White,
    Black,
}

impl Color {
    pub fn opp(self) -> Color {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }
    pub fn idx(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Piece {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

pub const PIECES: [Piece; 6] =
    [Piece::Pawn, Piece::Knight, Piece::Bishop, Piece::Rook, Piece::Queen, Piece::King];

pub type Cell = Option<(Color, Piece)>;

/// キャスリング権のビット: 白K=1, 白Q=2, 黒K=4, 黒Q=8
pub const WK: u8 = 1;
pub const WQ: u8 = 2;
pub const BK: u8 = 4;
pub const BQ: u8 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Move {
    pub from: u8,
    pub to: u8,
    pub promo: Option<Piece>,
}

impl Move {
    pub const NULL: Move = Move { from: 0, to: 0, promo: None };

    pub fn new(from: u8, to: u8, promo: Option<Piece>) -> Move {
        Move { from, to, promo }
    }

    /// "e2e4", "e7e8q" 形式
    pub fn to_uci(self) -> String {
        let mut s = format!("{}{}", sq_name(self.from), sq_name(self.to));
        if let Some(p) = self.promo {
            s.push(piece_char(Color::Black, p));
        }
        s
    }
}

impl fmt::Display for Move {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_uci())
    }
}

pub fn sq_name(sq: u8) -> String {
    let file = (b'a' + (sq & 7)) as char;
    let rank = (b'1' + (sq >> 3)) as char;
    format!("{file}{rank}")
}

pub fn parse_sq(s: &str) -> Option<u8> {
    let b = s.as_bytes();
    if b.len() != 2 {
        return None;
    }
    let (f, r) = (b[0].wrapping_sub(b'a'), b[1].wrapping_sub(b'1'));
    if f < 8 && r < 8 {
        Some(r * 8 + f)
    } else {
        None
    }
}

/// from / to がこのマスに触れたとき残るキャスリング権のマスク。
fn castle_mask(sq: u8) -> u8 {
    match sq {
        0 => !WQ,
        4 => !(WK | WQ),
        7 => !WK,
        56 => !BQ,
        60 => !(BK | BQ),
        63 => !BK,
        _ => 0xFF,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Position {
    /// 駒種ごとのビットボード（色は問わない）。`Piece as usize` で引く。
    pub pieces: [Bitboard; 6],
    /// 色ごとのビットボード。`Color as usize` で引く。
    pub colors: [Bitboard; 2],
    pub side: Color,
    pub castling: u8,
    /// アンパッサンで取れるマス（ダブルステップ直後のみ Some）
    pub ep: Option<u8>,
    pub halfmove: u16,
    pub fullmove: u16,
}

pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

impl Position {
    pub fn startpos() -> Position {
        Position::from_fen(START_FEN).unwrap()
    }

    pub fn from_fen(fen: &str) -> Result<Position, String> {
        let mut it = fen.split_whitespace();
        let placement = it.next().ok_or("FEN が空です")?;
        let side = it.next().unwrap_or("w");
        let castling = it.next().unwrap_or("-");
        let ep = it.next().unwrap_or("-");
        let halfmove = it.next().unwrap_or("0");
        let fullmove = it.next().unwrap_or("1");

        let mut pos = Position {
            pieces: [0; 6],
            colors: [0; 2],
            side: if side == "b" { Color::Black } else { Color::White },
            castling: 0,
            ep: parse_sq(ep),
            halfmove: halfmove.parse().unwrap_or(0),
            fullmove: fullmove.parse().unwrap_or(1),
        };

        let ranks: Vec<&str> = placement.split('/').collect();
        if ranks.len() != 8 {
            return Err("FEN の段数が 8 ではありません".into());
        }
        for (i, row) in ranks.iter().enumerate() {
            let rank = 7 - i as u32;
            let mut file = 0u32;
            for ch in row.chars() {
                if let Some(d) = ch.to_digit(10) {
                    file += d;
                    continue;
                }
                let color = if ch.is_ascii_uppercase() { Color::White } else { Color::Black };
                let piece = match ch.to_ascii_lowercase() {
                    'p' => Piece::Pawn,
                    'n' => Piece::Knight,
                    'b' => Piece::Bishop,
                    'r' => Piece::Rook,
                    'q' => Piece::Queen,
                    'k' => Piece::King,
                    _ => return Err(format!("不正な駒: {ch}")),
                };
                if file >= 8 {
                    return Err("FEN の筋が範囲外です".into());
                }
                let bit = 1u64 << (rank * 8 + file);
                pos.pieces[piece as usize] |= bit;
                pos.colors[color.idx()] |= bit;
                file += 1;
            }
        }
        for c in [Color::White, Color::Black] {
            if (pos.pieces[Piece::King as usize] & pos.colors[c.idx()]).count_ones() != 1 {
                return Err("キングは各色 1 枚必要です".into());
            }
        }

        for ch in castling.chars() {
            pos.castling |= match ch {
                'K' => WK,
                'Q' => WQ,
                'k' => BK,
                'q' => BQ,
                _ => 0,
            };
        }
        Ok(pos)
    }

    pub fn to_fen(&self) -> String {
        let mut s = String::new();
        for rank in (0..8).rev() {
            let mut empty = 0;
            for file in 0..8 {
                match self.piece_at(rank * 8 + file) {
                    None => empty += 1,
                    Some((c, p)) => {
                        if empty > 0 {
                            s.push_str(&empty.to_string());
                            empty = 0;
                        }
                        s.push(piece_char(c, p));
                    }
                }
            }
            if empty > 0 {
                s.push_str(&empty.to_string());
            }
            if rank > 0 {
                s.push('/');
            }
        }
        s.push(' ');
        s.push(if self.side == Color::White { 'w' } else { 'b' });
        s.push(' ');
        if self.castling == 0 {
            s.push('-');
        } else {
            for (bit, ch) in [(WK, 'K'), (WQ, 'Q'), (BK, 'k'), (BQ, 'q')] {
                if self.castling & bit != 0 {
                    s.push(ch);
                }
            }
        }
        s.push(' ');
        match self.ep {
            Some(sq) => s.push_str(&sq_name(sq)),
            None => s.push('-'),
        }
        s.push_str(&format!(" {} {}", self.halfmove, self.fullmove));
        s
    }

    #[inline]
    pub fn occupied(&self) -> Bitboard {
        self.colors[0] | self.colors[1]
    }

    /// 指定した色・駒種のビットボード
    #[inline]
    pub fn bb(&self, c: Color, p: Piece) -> Bitboard {
        self.pieces[p as usize] & self.colors[c.idx()]
    }

    #[inline]
    pub fn king_sq(&self, c: Color) -> u8 {
        self.bb(c, Piece::King).trailing_zeros() as u8
    }

    pub fn piece_at(&self, sq: u8) -> Cell {
        let bit = 1u64 << sq;
        if self.occupied() & bit == 0 {
            return None;
        }
        let color = if self.colors[0] & bit != 0 { Color::White } else { Color::Black };
        let piece = PIECES.into_iter().find(|&p| self.pieces[p as usize] & bit != 0)?;
        Some((color, piece))
    }

    /// `sq` が `by` 側の駒に攻撃されているか。
    /// 「sq に駒を置いたらどこに利くか」を逆に使い、そこに相手の該当駒がいるか調べる。
    #[inline]
    pub fn is_attacked(&self, sq: u8, by: Color) -> bool {
        self.is_attacked_with_occ(sq, by, self.occupied())
    }

    /// 占有ビットボードを差し替えて利きを調べる（キングを抜いた盤で判定する用）。
    #[inline]
    pub fn is_attacked_with_occ(&self, sq: u8, by: Color, occ: Bitboard) -> bool {
        let s = sq as usize;
        let them = self.colors[by.idx()];
        let p = &self.pieces;
        PAWN[by.opp().idx()][s] & p[Piece::Pawn as usize] & them != 0
            || KNIGHT[s] & p[Piece::Knight as usize] & them != 0
            || KING[s] & p[Piece::King as usize] & them != 0
            || bishop_attacks(s, occ) & (p[Piece::Bishop as usize] | p[Piece::Queen as usize]) & them != 0
            || rook_attacks(s, occ) & (p[Piece::Rook as usize] | p[Piece::Queen as usize]) & them != 0
    }

    pub fn in_check(&self) -> bool {
        self.is_attacked(self.king_sq(self.side), self.side.opp())
    }

    /// 指し手を適用した新しい局面を返す（合法性は検査しない）。
    pub fn make_move(&self, m: Move) -> Position {
        let mut p = *self;
        let us = self.side;
        let them = us.opp();
        let from_bit = 1u64 << m.from;
        let to_bit = 1u64 << m.to;

        let piece = PIECES
            .into_iter()
            .find(|&pc| self.pieces[pc as usize] & from_bit != 0)
            .expect("from に駒がありません");
        let captured = self.colors[them.idx()] & to_bit != 0;

        if captured {
            for pc in &mut p.pieces {
                *pc &= !to_bit;
            }
            p.colors[them.idx()] &= !to_bit;
        }

        p.pieces[piece as usize] &= !from_bit;
        p.pieces[m.promo.unwrap_or(piece) as usize] |= to_bit;
        p.colors[us.idx()] ^= from_bit | to_bit;

        let dist = m.to as i8 - m.from as i8;
        match piece {
            Piece::Pawn => {
                // アンパッサン: ep マスへ斜めに進む
                if Some(m.to) == self.ep && dist % 8 != 0 {
                    let cap = if us == Color::White { m.to - 8 } else { m.to + 8 };
                    let cap_bit = 1u64 << cap;
                    p.pieces[Piece::Pawn as usize] &= !cap_bit;
                    p.colors[them.idx()] &= !cap_bit;
                }
            }
            Piece::King if dist.abs() == 2 => {
                // キャスリング: ルークも動かす
                let (rf, rt) = if dist > 0 { (m.to + 1, m.to - 1) } else { (m.to - 2, m.to + 1) };
                let rook_bits = (1u64 << rf) | (1u64 << rt);
                p.pieces[Piece::Rook as usize] ^= rook_bits;
                p.colors[us.idx()] ^= rook_bits;
            }
            _ => {}
        }

        p.castling &= castle_mask(m.from) & castle_mask(m.to);
        p.ep = if piece == Piece::Pawn && dist.abs() == 16 {
            Some((m.from + m.to) / 2)
        } else {
            None
        };
        p.halfmove = if piece == Piece::Pawn || captured { 0 } else { p.halfmove + 1 };
        if us == Color::Black {
            p.fullmove += 1;
        }
        p.side = them;
        p
    }

    /// 盤面・手番・キャスリング権・アンパッサンが同じか（千日手判定用）
    pub fn same_position(&self, other: &Position) -> bool {
        self.side == other.side
            && self.castling == other.castling
            && self.ep == other.ep
            && self.pieces == other.pieces
            && self.colors == other.colors
    }
}

pub fn piece_char(c: Color, p: Piece) -> char {
    let ch = match p {
        Piece::Pawn => 'p',
        Piece::Knight => 'n',
        Piece::Bishop => 'b',
        Piece::Rook => 'r',
        Piece::Queen => 'q',
        Piece::King => 'k',
    };
    if c == Color::White {
        ch.to_ascii_uppercase()
    } else {
        ch
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for rank in (0..8u8).rev() {
            write!(f, "{} ", rank + 1)?;
            for file in 0..8u8 {
                match self.piece_at(rank * 8 + file) {
                    None => write!(f, ". ")?,
                    Some((c, p)) => write!(f, "{} ", piece_char(c, p))?,
                }
            }
            writeln!(f)?;
        }
        writeln!(f, "  a b c d e f g h")?;
        write!(f, "{}", self.to_fen())
    }
}
