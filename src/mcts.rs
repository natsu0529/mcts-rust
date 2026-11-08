//! 学習なしの純粋 MCTS。
//! - `flat_mc`: ルートの各手を均等にランダムプレイアウトして勝率で選ぶ
//! - `uct`: UCB1 で木を育てる UCT

use crate::board::{Color, Move, Position};
use crate::movegen::GameResult;
use crate::rng::Rng;

#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// 試行（プレイアウト）回数
    pub iterations: u32,
    /// UCB1 の探索定数
    pub c: f64,
    /// プレイアウトの最大手数。超えたら引き分け扱い。
    pub max_rollout_plies: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config { iterations: 100_000, c: 1.0, max_rollout_plies: 300 }
    }
}

#[derive(Clone, Debug)]
pub struct MoveStat {
    pub mv: Move,
    pub visits: u32,
    /// この手を指す側から見た勝率
    pub win_rate: f64,
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub best: Option<Move>,
    /// 訪問数の多い順
    pub stats: Vec<MoveStat>,
}

/// 終局までランダムに指し、白視点のスコアを返す。
pub fn rollout(start: &Position, rng: &mut Rng, max_plies: u32) -> f64 {
    rollout_with_len(start, rng, max_plies).0
}

/// `rollout` と同じだが、指した手数も返す（計測用）。
pub fn rollout_with_len(start: &Position, rng: &mut Rng, max_plies: u32) -> (f64, u32) {
    let mut pos = *start;
    for ply in 0..max_plies {
        if pos.halfmove >= 100 || pos.insufficient_material() {
            return (0.5, ply);
        }
        match pos.random_legal_move(rng) {
            Some(m) => pos = pos.make_move(m),
            None => {
                let score = if pos.in_check() {
                    match pos.side {
                        Color::White => 0.0,
                        Color::Black => 1.0,
                    }
                } else {
                    0.5
                };
                return (score, ply);
            }
        }
    }
    (0.5, max_plies)
}

/// `mover` 視点のスコアに直す。
fn score_for(white_score: f64, mover: Color) -> f64 {
    if mover == Color::White { white_score } else { 1.0 - white_score }
}

/// Flat Monte Carlo: 各手に試行を均等に配り、勝率最大の手を選ぶ。
pub fn flat_mc(root: &Position, cfg: &Config, rng: &mut Rng) -> SearchResult {
    let moves = root.legal_moves();
    let moves = moves.as_slice();
    if moves.is_empty() {
        return SearchResult { best: None, stats: vec![] };
    }
    let children: Vec<Position> = moves.iter().map(|&m| root.make_move(m)).collect();
    let mut visits = vec![0u32; moves.len()];
    let mut wins = vec![0.0f64; moves.len()];

    for i in 0..cfg.iterations as usize {
        let k = i % moves.len();
        let s = match children[k].result() {
            Some(r) => r.white_score(),
            None => rollout(&children[k], rng, cfg.max_rollout_plies),
        };
        visits[k] += 1;
        wins[k] += score_for(s, root.side);
    }

    let mut stats: Vec<MoveStat> = (0..moves.len())
        .map(|k| MoveStat {
            mv: moves[k],
            visits: visits[k],
            win_rate: wins[k] / visits[k].max(1) as f64,
        })
        .collect();
    stats.sort_by(|a, b| b.win_rate.partial_cmp(&a.win_rate).unwrap());
    SearchResult { best: stats.first().map(|s| s.mv), stats }
}

struct Node {
    pos: Position,
    mv: Option<Move>,
    parent: Option<usize>,
    children: Vec<usize>,
    untried: Vec<Move>,
    /// 終端なら白視点のスコア
    terminal: Option<f64>,
    visits: u32,
    /// このノードへ指した側（親の手番）から見た勝ち点の合計
    value: f64,
}

impl Node {
    fn new(pos: Position, mv: Option<Move>, parent: Option<usize>) -> Node {
        let legal = pos.legal_moves();
        let terminal = if legal.is_empty() {
            Some(pos.result().map_or(0.5, GameResult::white_score))
        } else if pos.halfmove >= 100 || pos.insufficient_material() {
            Some(0.5)
        } else {
            None
        };
        let untried = if terminal.is_some() { vec![] } else { legal.as_slice().to_vec() };
        Node { pos, mv, parent, children: vec![], untried, terminal, visits: 0, value: 0.0 }
    }
}

/// UCT: 選択（UCB1）→ 展開 → ランダムプレイアウト → 逆伝播。
pub fn uct(root: &Position, cfg: &Config, rng: &mut Rng) -> SearchResult {
    let mut nodes = vec![Node::new(*root, None, None)];
    if nodes[0].terminal.is_some() {
        return SearchResult { best: None, stats: vec![] };
    }

    for _ in 0..cfg.iterations {
        // 選択: 完全展開済みの間、UCB1 最大の子へ降りる
        let mut n = 0;
        while nodes[n].terminal.is_none() && nodes[n].untried.is_empty() {
            let ln_parent = (nodes[n].visits as f64).ln();
            let mut best = (f64::NEG_INFINITY, nodes[n].children[0]);
            for &ch in &nodes[n].children {
                let c = &nodes[ch];
                let v = c.visits as f64;
                let ucb = c.value / v + cfg.c * (ln_parent / v).sqrt();
                if ucb > best.0 {
                    best = (ucb, ch);
                }
            }
            n = best.1;
        }

        // 展開
        if nodes[n].terminal.is_none() {
            let i = rng.below(nodes[n].untried.len());
            let m = nodes[n].untried.swap_remove(i);
            let child = Node::new(nodes[n].pos.make_move(m), Some(m), Some(n));
            nodes.push(child);
            let id = nodes.len() - 1;
            nodes[n].children.push(id);
            n = id;
        }

        // シミュレーション
        let white_score = match nodes[n].terminal {
            Some(s) => s,
            None => rollout(&nodes[n].pos, rng, cfg.max_rollout_plies),
        };

        // 逆伝播
        let mut cur = Some(n);
        while let Some(i) = cur {
            let mover = nodes[i].pos.side.opp(); // このノードへ指した側
            nodes[i].visits += 1;
            nodes[i].value += score_for(white_score, mover);
            cur = nodes[i].parent;
        }
    }

    let mut stats: Vec<MoveStat> = nodes[0]
        .children
        .iter()
        .map(|&ch| MoveStat {
            mv: nodes[ch].mv.unwrap(),
            visits: nodes[ch].visits,
            win_rate: nodes[ch].value / nodes[ch].visits as f64,
        })
        .collect();
    stats.sort_by(|a, b| b.visits.cmp(&a.visits));
    SearchResult { best: stats.first().map(|s| s.mv), stats }
}
