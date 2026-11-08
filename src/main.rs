use mcts_chess::board::{Color, Move, Position, START_FEN};
use mcts_chess::mcts::{flat_mc, rollout_with_len, uct, Config, SearchResult};
use mcts_chess::movegen::GameResult;
use mcts_chess::rng::Rng;
use std::io::{self, BufRead, Write};
use std::time::Instant;

const USAGE: &str = "使い方:
  mcts_chess perft <depth> [fen]            perft（ルール検証・速度測定）
  mcts_chess best <iters> [flat] [fen]      最善手を探索して表示（既定は UCT）
  mcts_chess vs-random <games> <iters>      UCT 対 ランダムプレイヤーの対局
  mcts_chess vs-flat <games> <iters>        UCT 対 Flat MC の対局
  mcts_chess vs-self <games> <iters A> <threads A> <iters B> <threads B>
                                            UCT 同士（設定違い）の対局
  mcts_chess play <iters>                   人間(白) 対 UCT(黒)。手は e2e4 形式";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("perft") => cmd_perft(&args[1..]),
        Some("best") => cmd_best(&args[1..]),
        Some("vs-random") => cmd_vs(&args[1..], false),
        Some("vs-flat") => cmd_vs(&args[1..], true),
        Some("vs-self") => cmd_vs_self(&args[1..]),
        Some("play") => cmd_play(&args[1..]),
        Some("bench") => cmd_bench(&args[1..]),
        _ => println!("{USAGE}"),
    }
}

fn fen_from(args: &[String]) -> Position {
    if args.is_empty() {
        Position::startpos()
    } else {
        Position::from_fen(&args.join(" ")).expect("FEN が不正です")
    }
}

/// 固定シードで初期局面からのプレイアウトを繰り返し、速度を測る。
fn cmd_bench(args: &[String]) {
    let n: u32 = args.first().and_then(|s| s.parse().ok()).unwrap_or(20_000);
    let pos = Position::startpos();
    let mut rng = Rng::new(42);
    let (mut plies, mut decisive) = (0u64, 0u32);
    let t = Instant::now();
    for _ in 0..n {
        let (score, len) = rollout_with_len(&pos, &mut rng, Config::default().max_rollout_plies);
        plies += len as u64;
        if score != 0.5 {
            decisive += 1;
        }
    }
    let secs = t.elapsed().as_secs_f64();
    println!(
        "{n} playouts: {:.2}s, {:.0} playouts/s, {:.2}M plies/s, 平均 {:.1} 手, 決着率 {:.1}%",
        secs,
        n as f64 / secs,
        plies as f64 / secs / 1e6,
        plies as f64 / n as f64,
        decisive as f64 * 100.0 / n as f64
    );
    let t = Instant::now();
    let nodes = pos.perft(5);
    let secs = t.elapsed().as_secs_f64();
    println!("perft(5) = {nodes}: {:.2}s, {:.1}M nps", secs, nodes as f64 / secs / 1e6);
}

fn cmd_perft(args: &[String]) {
    let depth: u32 = args.first().and_then(|s| s.parse().ok()).unwrap_or(5);
    let pos = fen_from(args.get(1..).unwrap_or(&[]));
    let t = Instant::now();
    let n = pos.perft(depth);
    let secs = t.elapsed().as_secs_f64();
    println!("perft({depth}) = {n}  ({:.2}s, {:.1}M nps)", secs, n as f64 / secs / 1e6);
}

fn print_result(pos: &Position, r: &SearchResult, secs: f64, iters: u32) {
    println!("{pos}\n");
    for s in r.stats.iter().take(8) {
        println!("  {:6}  visits={:7}  win={:.3}", s.mv.to_uci(), s.visits, s.win_rate);
    }
    match r.best {
        Some(m) => println!(
            "bestmove {m}   ({iters} playouts, {:.2}s, {:.0} playouts/s)",
            secs,
            iters as f64 / secs
        ),
        None => println!("指せる手がありません"),
    }
}

fn cmd_best(args: &[String]) {
    let iters: u32 = args.first().and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let (flat, rest) = match args.get(1).map(String::as_str) {
        Some("flat") => (true, &args[2..]),
        _ => (false, args.get(1..).unwrap_or(&[])),
    };
    let pos = fen_from(rest);
    let cfg = Config { iterations: iters, ..Config::default() };
    let mut rng = Rng::from_time();
    let t = Instant::now();
    let r = if flat { flat_mc(&pos, &cfg, &mut rng) } else { uct(&pos, &cfg, &mut rng) };
    print_result(&pos, &r, t.elapsed().as_secs_f64(), iters);
}

#[derive(Clone, Copy)]
enum Player {
    Uct(Config),
    Flat(Config),
    Random,
}

impl Player {
    fn choose(&self, pos: &Position, rng: &mut Rng) -> Option<Move> {
        match self {
            Player::Uct(cfg) => uct(pos, cfg, rng).best,
            Player::Flat(cfg) => flat_mc(pos, cfg, rng).best,
            Player::Random => pos.random_legal_move(rng),
        }
    }
}

/// 1 局指して結果を返す。千日手（3 回）と 300 手打ち切りは引き分け。
fn play_game(white: &Player, black: &Player, rng: &mut Rng) -> GameResult {
    let mut pos = Position::from_fen(START_FEN).unwrap();
    let mut history = vec![pos];
    for _ in 0..300 {
        if let Some(r) = pos.result() {
            return r;
        }
        if history.iter().filter(|h| h.same_position(&pos)).count() >= 3 {
            return GameResult::Draw;
        }
        let player = if pos.side == Color::White { white } else { black };
        let Some(m) = player.choose(&pos, rng) else { return GameResult::Draw };
        pos = pos.make_move(m);
        history.push(pos);
    }
    GameResult::Draw
}

/// A と B を先後交互に games 局対局させ、A から見た勝敗を表示する。
fn run_match(name_a: &str, a: Player, name_b: &str, b: Player, games: u32) {
    let mut rng = Rng::from_time();
    let (mut w, mut d, mut l) = (0, 0, 0);
    for g in 0..games {
        let a_white = g % 2 == 0;
        let r = if a_white { play_game(&a, &b, &mut rng) } else { play_game(&b, &a, &mut rng) };
        let a_score = if a_white { r.white_score() } else { 1.0 - r.white_score() };
        let label = if a_score == 1.0 {
            w += 1;
            "A win"
        } else if a_score == 0.0 {
            l += 1;
            "A loss"
        } else {
            d += 1;
            "draw"
        };
        println!("game {:3}: A={} -> {label}", g + 1, if a_white { "White" } else { "Black" });
        io::stdout().flush().ok();
    }
    let score = (w as f64 + 0.5 * d as f64) / games as f64;
    println!("A: {name_a}\nB: {name_b}\nA の成績: +{w} ={d} -{l}  得点率 {:.1}%", score * 100.0);
}

fn arg<T: std::str::FromStr>(args: &[String], i: usize, default: T) -> T {
    args.get(i).and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// UCT（1 スレッド）対 ランダム / Flat MC
fn cmd_vs(args: &[String], vs_flat: bool) {
    let games: u32 = arg(args, 0, 10);
    let iters: u32 = arg(args, 1, 10_000);
    let cfg = Config { iterations: iters, threads: 1, ..Config::default() };
    let (name_b, b) = if vs_flat { ("Flat MC", Player::Flat(cfg)) } else { ("Random", Player::Random) };
    run_match(&format!("UCT {iters} playouts"), Player::Uct(cfg), name_b, b, games);
}

/// UCT 同士で、プレイアウト数とスレッド数を変えて対局させる。
fn cmd_vs_self(args: &[String]) {
    let games: u32 = arg(args, 0, 10);
    let ia: u32 = arg(args, 1, 20_000);
    let ta: usize = arg(args, 2, 1);
    let ib: u32 = arg(args, 3, 2_000);
    let tb: usize = arg(args, 4, 1);
    let a = Config { iterations: ia, threads: ta, ..Config::default() };
    let b = Config { iterations: ib, threads: tb, ..Config::default() };
    run_match(
        &format!("UCT {ia} playouts / {ta} threads"),
        Player::Uct(a),
        &format!("UCT {ib} playouts / {tb} threads"),
        Player::Uct(b),
        games,
    );
}

fn cmd_play(args: &[String]) {
    let iters: u32 = args.first().and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let cfg = Config { iterations: iters, ..Config::default() };
    let mut rng = Rng::from_time();
    let mut pos = Position::startpos();
    let stdin = io::stdin();
    loop {
        println!("\n{pos}");
        if let Some(r) = pos.result() {
            println!("終局: {r:?}");
            return;
        }
        if pos.side == Color::White {
            print!("あなたの手 > ");
            io::stdout().flush().ok();
            let mut line = String::new();
            if stdin.lock().read_line(&mut line).unwrap() == 0 {
                return;
            }
            match pos.parse_move(&line) {
                Some(m) => pos = pos.make_move(m),
                None => println!("不正な手です（例: e2e4, e7e8q）"),
            }
        } else {
            let t = Instant::now();
            let r = uct(&pos, &cfg, &mut rng);
            let m = r.best.unwrap();
            println!("エンジンの手: {m}  (勝率 {:.3}, {:.1}s)", r.stats[0].win_rate, t.elapsed().as_secs_f64());
            pos = pos.make_move(m);
        }
    }
}
