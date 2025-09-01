use std::hash::Hash;
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;

use board_game::board::{AltBoard, Board};
use board_game::games::arimaa::ArimaaBoard;
use board_game::games::chess::ChessBoard;
use board_game::games::sttt::STTTBoard;
use board_game::games::ttt::TTTBoard;
use clap::Parser;
use crossbeam::thread::Scope;
use flume::{Receiver, Sender};
use itertools::Itertools;
use kn_cuda_sys::wrapper::handle::CudaDevice;
use rand::rngs::StdRng;
use trictrac_bot::trictrac_board::TrictracBoard;

use kz_core::mapping::arimaa::ArimaaSplitMapper;
use kz_core::mapping::ataxx::AtaxxStdMapper;
use kz_core::mapping::chess::{ChessHistoryMapper, ChessStdMapper};
use kz_core::mapping::go::GoStdMapper;
use kz_core::mapping::sttt::STTTStdMapper;
use kz_core::mapping::trictrac::TrictracStdMapper;
use kz_core::mapping::ttt::TTTStdMapper;
use kz_core::mapping::BoardMapper;
use kz_core::network::dummy::NetworkOrDummy;
use kz_util::game::Game;

use crate::server::collector::collector_main;
use crate::server::commander::{commander_main, read_command};
use crate::server::protocol::{Command, GeneratorUpdate, Settings, StartupSettings};
use crate::server::server_alphazero::AlphaZeroSpecialization;
#[cfg(feature = "muzero")]
use crate::server::server_muzero::MuZeroSpecialization;
use crate::server::start_pos::{ataxx_start_pos, go_start_pos};

#[derive(Debug, clap::Parser)]
struct Args {
    #[clap(short, long)]
    port: Option<u16>,
    #[clap(short, long)]
    device: Vec<i32>,
}

pub fn selfplay_server_main() {
    let args: Args = Args::parse();

    let devices = if args.device.is_empty() {
        CudaDevice::all().collect_vec()
    } else {
        args.device.iter().map(|&d| CudaDevice::new(d).unwrap()).collect_vec()
    };
    assert!(!devices.is_empty(), "No cuda devices found");
    println!("Using devices: {:?}", devices);

    let port = args.port.unwrap_or(63105);
    println!("Waiting for connection on port {}", port);
    let (stream, addr) = TcpListener::bind(("127.0.0.1", port)).unwrap().accept().unwrap();
    println!("Accepted connection {:?} on {:?}", stream, addr);

    let writer = BufWriter::new(&stream);
    let mut reader = BufReader::new(&stream);

    let startup_settings = wait_for_startup_settings(&mut reader);
    println!("Received startup settings:\n{:#?}", startup_settings);

    assert_ne!(startup_settings.gpu_batch_size, 0, "GPU batch size cannot be 0");
    assert_ne!(startup_settings.search_batch_size, 0, "Search batch size cannot be 0");
    assert!(
        startup_settings.gpu_batch_size >= startup_settings.search_batch_size,
        "It's not useful to have a GPU batch size smaller than the search batch size"
    );

    if startup_settings.muzero {
        assert_ne!(
            startup_settings.gpu_batch_size_root, 0,
            "For muzero root batch size must be nonzero"
        );
        assert_eq!(
            startup_settings.search_batch_size, 1,
            "For muzero search batch size must be 1, minibatching is not yet supported"
        )
    }

    let output_folder = Path::new(&startup_settings.output_folder);
    assert!(
        output_folder.exists(),
        "Output folder does not exist, got '{}'",
        startup_settings.output_folder
    );
    assert!(
        output_folder.is_absolute(),
        "Output folder is not an absolute path, got '{}'",
        startup_settings.output_folder
    );

    let game =
        Game::parse(&startup_settings.game).unwrap_or_else(|| panic!("Unknown game '{}'", startup_settings.game));

    selfplay_start_dispatch_game(game, devices, startup_settings, writer, reader)
}

fn selfplay_start_dispatch_game(
    game: Game,
    devices: Vec<CudaDevice>,
    startup_settings: StartupSettings,
    writer: BufWriter<&TcpStream>,
    reader: BufReader<&TcpStream>,
) {
    //TODO static dispatch this early means we're generating a lot of code N times
    //  is it actually that much? -> investigate with objdump or similar
    //  would it be relatively easy to delay this dispatch some more?
    match game {
        Game::TTT => {
            assert_eq!(startup_settings.start_pos, "default");
            selfplay_start_dispatch_spec_alt(
                game,
                devices,
                startup_settings,
                |_| TTTBoard::default(),
                TTTStdMapper,
                reader,
                writer,
            )
        }
        Game::STTT => {
            assert_eq!(startup_settings.start_pos, "default");
            selfplay_start_dispatch_spec_alt(
                game,
                devices,
                startup_settings,
                |_| STTTBoard::default(),
                STTTStdMapper,
                reader,
                writer,
            )
        }
        Game::Ataxx { size } => {
            let start_pos = ataxx_start_pos(size, &startup_settings.start_pos);
            selfplay_start_dispatch_spec_alt(
                game,
                devices,
                startup_settings,
                start_pos,
                AtaxxStdMapper::new(size),
                reader,
                writer,
            )
        }
        Game::Chess => {
            assert_eq!(startup_settings.start_pos, "default");
            selfplay_start_dispatch_spec_alt(
                game,
                devices,
                startup_settings,
                |_| ChessBoard::default(),
                ChessStdMapper,
                reader,
                writer,
            )
        }
        Game::ChessHist { length } => {
            assert_eq!(startup_settings.start_pos, "default");
            selfplay_start_dispatch_spec_alt(
                game,
                devices,
                startup_settings,
                |_| ChessBoard::default(),
                ChessHistoryMapper::new(length),
                reader,
                writer,
            )
        }
        Game::ArimaaSplit => {
            assert_eq!(startup_settings.start_pos, "default");
            selfplay_start_dispatch_spec_non_alt(
                game,
                devices,
                startup_settings,
                |_| ArimaaBoard::default(),
                ArimaaSplitMapper,
                reader,
                writer,
            )
        }
        Game::Trictrac => {
            assert_eq!(startup_settings.start_pos, "default");
            selfplay_start_dispatch_spec_non_alt(
                game,
                devices,
                startup_settings,
                |_| TrictracBoard::default(),
                TrictracStdMapper,
                reader,
                writer,
            )
        }
        Game::Go { size } => {
            let start_pos = go_start_pos(size, &startup_settings.start_pos);
            selfplay_start_dispatch_spec_non_alt(
                game,
                devices,
                startup_settings,
                start_pos,
                GoStdMapper::new(size, true),
                reader,
                writer,
            )
        }
    }
}

fn selfplay_start_dispatch_spec_alt<
    B: AltBoard + Hash,
    M: BoardMapper<B> + 'static,
    F: Fn(&mut StdRng) -> B + Send + Sync + Clone + 'static,
>(
    game: Game,
    devices: Vec<CudaDevice>,
    startup: StartupSettings,
    start_pos: F,
    mapper: M,
    reader: BufReader<impl Read + Send>,
    writer: BufWriter<impl Write + Send>,
) {
    if startup.muzero {
        #[cfg(feature = "muzero")]
        selfplay_start(
            game,
            devices,
            startup,
            mapper,
            start_pos,
            reader,
            writer,
            MuZeroSpecialization,
        );

        #[cfg(not(feature = "muzero"))]
        panic!("MuZero feature was not enabled")
    } else {
        selfplay_start(
            game,
            devices,
            startup,
            mapper,
            start_pos,
            reader,
            writer,
            AlphaZeroSpecialization,
        );
    }
}

fn selfplay_start_dispatch_spec_non_alt<
    B: Board + Hash,
    M: BoardMapper<B> + 'static,
    F: Fn(&mut StdRng) -> B + Send + Sync + Clone + 'static,
>(
    game: Game,
    devices: Vec<CudaDevice>,
    startup: StartupSettings,
    start_pos: F,
    mapper: M,
    reader: BufReader<impl Read + Send>,
    writer: BufWriter<impl Write + Send>,
) {
    if startup.muzero {
        panic!("MuZero only supports alternating boards for now");
    } else {
        selfplay_start(
            game,
            devices,
            startup,
            mapper,
            start_pos,
            reader,
            writer,
            AlphaZeroSpecialization,
        );
    }
}

fn wait_for_startup_settings(reader: &mut BufReader<&TcpStream>) -> StartupSettings {
    match read_command(reader) {
        Command::StartupSettings(startup) => startup,
        command => panic!(
            "Must receive startup settings before any other command, got {:?}",
            command
        ),
    }
}

pub type UpdateSender<B> = Sender<GeneratorUpdate<B>>;
pub type GraphMessage<G> = Option<NetworkOrDummy<Arc<G>>>;
pub type GraphSender<G> = Sender<GraphMessage<G>>;
pub type GraphReceiver<G> = Receiver<GraphMessage<G>>;

pub trait ZeroSpecialization<B: Board, M: BoardMapper<B> + 'static> {
    type G: Send + Sync;

    fn spawn_device_threads<'s>(
        &self,
        s: &Scope<'s>,
        device: CudaDevice,
        device_id: usize,
        startup: &StartupSettings,
        mapper: M,
        start_pos: impl Fn(&mut StdRng) -> B + Send + Sync + Clone + 'static,
        update_sender: UpdateSender<B>,
    ) -> (Vec<Sender<Settings>>, Vec<GraphSender<Self::G>>);

    fn load_graph(&self, path: &str, mapper: M, startup: &StartupSettings) -> Self::G;
}

fn selfplay_start<B: Board, M: BoardMapper<B> + 'static, Z: ZeroSpecialization<B, M> + Send + Sync>(
    game: Game,
    devices: Vec<CudaDevice>,
    startup: StartupSettings,
    mapper: M,
    start_pos: impl Fn(&mut StdRng) -> B + Send + Sync + Clone + 'static,
    reader: BufReader<impl Read + Send>,
    writer: BufWriter<impl Write + Send>,
    spec: Z,
) {
    assert!(!devices.is_empty());

    let total_cpu_threads = startup.cpu_threads_per_device * devices.len();
    let startup = &startup;

    let mut settings_senders = vec![];
    let mut graph_senders: Vec<GraphSender<Z::G>> = vec![];
    let (update_sender, update_receiver) = flume::bounded(total_cpu_threads);

    crossbeam::scope(|s| {
        // spawn per-device threads
        for (device_id, &device) in devices.iter().enumerate() {
            let start_pos = start_pos.clone();
            let (mut new_settings_senders, mut new_graph_senders) =
                spec.spawn_device_threads(s, device, device_id, &startup, mapper, start_pos, update_sender.clone());
            settings_senders.append(&mut new_settings_senders);
            graph_senders.append(&mut new_graph_senders);
        }

        // spawn collector
        s.builder()
            .name("collector".to_string())
            .spawn(move |_| {
                collector_main(
                    &game.to_string(),
                    writer,
                    startup.muzero,
                    startup.games_per_gen,
                    startup.first_gen,
                    &startup.output_folder,
                    mapper,
                    update_receiver,
                )
            })
            .unwrap();

        // spawn commander
        s.builder()
            .name("commander".to_string())
            .spawn(move |_| {
                commander_main(reader, settings_senders, graph_senders, update_sender, |path| {
                    spec.load_graph(path, mapper, startup)
                });
            })
            .unwrap();

        // implicitly join all spawned threads
    })
    .unwrap();
}
