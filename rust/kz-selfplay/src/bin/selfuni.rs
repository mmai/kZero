use std::borrow::Cow;
use std::path::PathBuf;

use board_game::ai::solver::solve_all_moves;
use board_game::board::Board;
use board_game::games::arimaa::ArimaaBoard;
use board_game::games::ataxx::AtaxxBoard;
use board_game::games::chess::ChessBoard;
use board_game::games::go::{GoBoard, Komi, Rules};
use board_game::games::sttt::STTTBoard;
use board_game::games::ttt::TTTBoard;
use board_game::wdl::OutcomeWDL;
use clap::Parser;
use flume::{Receiver, Sender};
use internal_iterator::InternalIterator;
use kz_core::mapping::arimaa::ArimaaSplitMapper;
use rand::seq::SliceRandom;
use rand::thread_rng;
use trictrac_bot::trictrac_board::TrictracBoard;

use kz_core::mapping::ataxx::AtaxxStdMapper;
use kz_core::mapping::chess::{ChessHistoryMapper, ChessStdMapper};
use kz_core::mapping::go::GoStdMapper;
use kz_core::mapping::sttt::STTTStdMapper;
use kz_core::mapping::trictrac::TrictracStdMapper;
use kz_core::mapping::ttt::TTTStdMapper;
use kz_core::mapping::BoardMapper;
use kz_core::network::dummy::{uniform_policy, uniform_values};
use kz_core::network::ZeroEvaluation;
use kz_core::zero::values::ZeroValuesPov;
use kz_selfplay::binary_output::BinaryOutput;
use kz_selfplay::simulation::{Position, Simulation};
use kz_util::game::Game;
use kz_util::throughput::PrintThroughput;

#[derive(Debug, Parser)]
struct Args {
    #[clap(long)]
    game: String,
    #[clap(long)]
    game_count: u64,
    #[clap(long)]
    max_game_length: u64,

    #[clap(long)]
    solver_depth: Option<u32>,
    #[clap(long)]
    thread_count: Option<u32>,

    bin_path: PathBuf,
}

fn main() {
    let mut args: Args = Args::parse();

    let game = Game::parse(&args.game).unwrap();
    args.game = game.to_string();

    match game {
        Game::TTT => main_impl(&args, TTTBoard::default, TTTStdMapper),
        Game::STTT => main_impl(&args, STTTBoard::default, STTTStdMapper),
        Game::Chess => main_impl(&args, ChessBoard::default, ChessStdMapper),
        Game::Trictrac => main_impl(&args, TrictracBoard::default, TrictracStdMapper),
        Game::Ataxx { size } => main_impl(&args, || AtaxxBoard::diagonal(size), AtaxxStdMapper::new(size)),
        Game::ChessHist { length } => main_impl(&args, ChessBoard::default, ChessHistoryMapper::new(length)),
        Game::ArimaaSplit => main_impl(&args, ArimaaBoard::default, ArimaaSplitMapper),
        Game::Go { size } => main_impl(
            &args,
            || GoBoard::new(size, Komi::new(15), Rules::cgos()),
            GoStdMapper::new(size, true),
        ),
    }
}

fn main_impl<B: Board, M: BoardMapper<B>>(args: &Args, start_pos: impl Fn() -> B + Sync, mapper: M) {
    let thread_count = args.thread_count.unwrap_or(1) as usize;

    crossbeam::scope(|s| {
        let (sender, receiver) = flume::bounded(2 * thread_count);

        // spawn generators
        for i in 0..thread_count {
            let sender = sender.clone();

            s.builder()
                .name(format!("generator-{}", i))
                .spawn(|_| {
                    main_generator(args, &start_pos, sender);
                })
                .unwrap();
        }

        // run collection in main thread
        main_collector(args, mapper, receiver);
    })
    .unwrap();
}

fn main_collector<B: Board, M: BoardMapper<B>>(args: &Args, mapper: M, receiver: Receiver<Simulation<B>>) {
    let mut output = BinaryOutput::new(&args.bin_path, &args.game, mapper).unwrap();
    let mut tp = PrintThroughput::new("games");

    for _ in 0..args.game_count {
        let sim = receiver.recv().unwrap();
        output.append(&sim).unwrap();
        tp.update_delta(1);
    }

    output.finish().unwrap();
    drop(receiver);
}

fn main_generator<B: Board>(args: &Args, start_pos: impl Fn() -> B, sender: Sender<Simulation<B>>) {
    let mut rng = thread_rng();
    let solver_depth = args.solver_depth.unwrap_or(0);

    loop {
        let mut board = start_pos();
        let mut positions = vec![];

        for _ in 0..args.max_game_length {
            if board.is_done() {
                break;
            }

            let net_eval = ZeroEvaluation {
                values: uniform_values(),
                policy: Cow::Owned(uniform_policy(board.available_moves().unwrap().count())),
            };

            let solution = solve_all_moves(&board, solver_depth);
            let (zero_eval, mv) = if let Some(moves) = solution.best_move {
                let outcome = solution.value.to_outcome_wdl().unwrap_or(OutcomeWDL::Draw);
                let policy = board
                    .available_moves()
                    .unwrap()
                    .map(|mv: B::Move| moves.contains(&mv) as u8 as f32 / moves.len() as f32)
                    .collect();

                let zero_eval = ZeroEvaluation {
                    values: ZeroValuesPov::from_outcome(outcome, 0.0),
                    policy: Cow::Owned(policy),
                };

                let mv = *moves.choose(&mut rng).unwrap();

                (zero_eval, mv)
            } else {
                (net_eval.clone(), board.random_available_move(&mut rng).unwrap())
            };

            let position = Position {
                board: board.clone(),
                is_full_search: true,
                played_mv: mv,
                zero_visits: 0,
                zero_evaluation: zero_eval,
                net_evaluation: net_eval,
            };
            positions.push(position);

            board.play(mv).unwrap();
        }

        let sim = Simulation {
            positions,
            final_board: board,
        };

        match sender.send(sim) {
            Ok(()) => {}
            Err(_) => break,
        }
    }
}
