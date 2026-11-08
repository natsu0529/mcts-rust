# mcts-rust

学習なし（policy / value なし）の純粋 MCTS で指すチェスエンジン。ルールもゼロから実装しています（外部クレートなし）。

## 構成

- `src/attacks.rs` — 利きテーブル（コンパイル時に計算）と飛び駒の利き
- `src/board.rs` — 盤面（ビットボード）、FEN、利き判定、指し手の適用
- `src/movegen.rs` — 指し手生成、終局判定、perft とそのテスト
- `src/mcts.rs` — Flat Monte Carlo と UCT
- `src/rng.rs` — xorshift64* 乱数
- `src/main.rs` — CLI

## 使い方

```sh
cargo test --release                              # perft でルールを検証
cargo run --release -- perft 5                    # perft(5) = 4865609
cargo run --release -- best 100000                # 初期局面を UCT で探索
cargo run --release -- best 100000 flat           # Flat MC で探索
cargo run --release -- best 20000 6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1
cargo run --release -- vs-random 10 2000          # UCT 対 ランダム
cargo run --release -- vs-flat 10 5000            # UCT 対 Flat MC
cargo run --release -- play 50000                 # 人間(白) 対 UCT(黒)
cargo run --release -- bench 100000               # プレイアウト速度と perft 速度の計測（固定シード）
```
