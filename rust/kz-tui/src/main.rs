use std::cell::RefCell;
use std::cmp::{max, min, Reverse};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use board_game::board::{Board, Outcome, Player};
use board_game::games::ataxx::AtaxxBoard;
use board_game::games::chess::{ChessBoard, Rules};
use board_game::games::go;
use board_game::games::go::{GoBoard, Komi};
use clap::Parser;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use decorum::N32;
use itertools::Itertools;
use kn_graph::onnx::load_graph_from_onnx_path;
use kn_graph::optimizer::optimize_graph;
use kn_runtime::{compiled_with_cuda_support, Device};
use rand::rngs::StdRng;
use rand::{thread_rng, SeedableRng};
use trictrac_bot::trictrac_board::TrictracBoard;
use tui::backend::CrosstermBackend;
use tui::buffer::Buffer;
use tui::layout::{Margin, Rect};
use tui::style::{Color, Modifier, Style};
use tui::widgets::Widget;
use tui::Terminal;

use kz_core::mapping::ataxx::AtaxxStdMapper;
use kz_core::mapping::chess::ChessStdMapper;
use kz_core::mapping::go::GoStdMapper;
use kz_core::mapping::trictrac::TrictracStdMapper;
use kz_core::mapping::BoardMapper;
use kz_core::network::prepared::PreparedNetwork;
use kz_core::network::symmetry::RandomSymmetryNetwork;
use kz_core::network::Network;
use kz_core::zero::node::{Uct, UctWeights};
use kz_core::zero::step::{zero_step_apply, zero_step_gather, FpuMode, QMode, ZeroRequest};
use kz_core::zero::tree::Tree;
use kz_core::zero::values::ZeroValuesAbs;
use kz_core::zero::wrapper::ZeroSettings;
use kz_util::display::display_option_empty;
use kz_util::game::Game;

#[derive(clap::Parser)]
struct Args {
    // game
    #[clap(long)]
    game: String,
    #[clap(long)]
    fen: Option<String>,

    // network
    #[clap(long)]
    network: PathBuf,
    #[clap(long)]
    cpu: bool,
    #[clap(long)]
    random_symmetries: bool,

    // search
    #[clap(long, default_value_t = 1)]
    batch_size: usize,
    #[clap(long, default_value_t = 1.0)]
    virtual_loss_weight: f32,
    #[clap(long, default_value_t = 1.0)]
    policy_temperature: f32,

    #[clap(long, default_value_t = 0)]
    visits: u64,
}

#[derive(Debug)]
struct State<B: Board> {
    settings: ZeroSettings,
    rng: StdRng,

    tree: Tree<B>,

    board_cache: RefCell<HashMap<usize, B>>,
    prev_nodes: Vec<RenderNode>,

    expanded_nodes: HashSet<usize>,
    selected_node: usize,

    view_offset: usize,
}

#[derive(Debug, Copy, Clone)]
struct RenderNode {
    node: usize,
    depth: u32,
}

fn main() -> std::io::Result<()> {
    let args: Args = Args::parse();

    let game = Game::parse(&args.game).expect("Invalid game");

    match game {
        Game::Chess => {
            let board = args.fen.as_ref().map_or(ChessBoard::default(), |fen| {
                ChessBoard::new_without_history_fen(fen, Rules::default())
            });
            main_game(&args, board, ChessStdMapper)
        }
        Game::Trictrac => {
            let board = args.fen.as_ref().map_or(TrictracBoard::default(), |fen| {
                TrictracBoard::from_fen(fen).expect("Invalid fen")
            });
            main_game(&args, board, TrictracStdMapper)
        }
        Game::Ataxx { size } => {
            let board = args.fen.as_ref().map_or(AtaxxBoard::diagonal(size), |fen| {
                AtaxxBoard::from_fen(fen).expect("Invalid fen")
            });
            assert_eq!(board.size(), size, "Fen has wrong size");
            main_game(&args, board, AtaxxStdMapper::new(size))
        }
        Game::Go { size } => {
            let komi = Komi::try_from(7.5).unwrap();
            let rules = go::Rules::tromp_taylor();
            let board = args.fen.as_ref().map_or(GoBoard::new(size, komi, rules), |fen| {
                GoBoard::from_fen(fen, rules).expect("Invalid fen")
            });
            assert_eq!(board.size(), size, "Fen has wrong size");
            main_game(&args, board, GoStdMapper::new(size, false))
        }

        _ => panic!("Game {game:?} not implemented yet"),
    }
}

fn main_game<B: Board, M: BoardMapper<B>>(args: &Args, board: B, mapper: M) -> std::io::Result<()> {
    println!("Using board:");
    println!("{}", board);
    println!("Using mapper: {:?}", mapper);

    println!("Loading graph...");
    let graph = load_graph_from_onnx_path(&args.network, true).unwrap();
    println!("Optimizing graph...");
    let graph = optimize_graph(&graph, Default::default());

    println!("Building network...");
    let device = if args.cpu {
        println!("Using CPU");
        Device::Cpu
    } else {
        if !compiled_with_cuda_support() {
            eprintln!("Warning: Not compiled with cuda support");
        }
        Device::best()
    };
    println!("Using device {:?}", device);

    let network_inner = PreparedNetwork::new(mapper, device, graph, args.batch_size);
    let mut network = RandomSymmetryNetwork::new(network_inner, thread_rng(), args.random_symmetries);

    // TODO expose as params?
    let settings = ZeroSettings::new(
        args.batch_size,
        UctWeights::default(),
        QMode::wdl(),
        FpuMode::Fixed(1.0),
        FpuMode::Fixed(0.0),
        args.virtual_loss_weight,
        args.policy_temperature,
    );

    main_impl(&mut network, board, settings, args.visits)
}

fn main_impl<B: Board>(
    network: &mut impl Network<B>,
    board: B,
    settings: ZeroSettings,
    visits: u64,
) -> std::io::Result<()> {
    // initialize state
    let mut rng = StdRng::from_entropy();
    println!("Building initial tree");
    let tree = settings.build_tree(&board, network, &mut rng, |tree| tree.root_visits() >= visits);

    println!(
        "nodes: {}, values: {:?}, depth {:?}",
        tree.root_visits(),
        tree.values(),
        tree.depth_range(0)
    );
    // return Ok(());

    let mut requests = VecDeque::new();
    let mut state = State {
        tree,
        settings,
        prev_nodes: Default::default(),
        board_cache: Default::default(),
        expanded_nodes: Default::default(),
        selected_node: 0,
        view_offset: 0,
        rng,
    };
    state.expanded_nodes.insert(0);

    // setup terminal
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // event loop
    loop {
        let mut prev_area = None;

        terminal.draw(|f| {
            let area = f.size().inner(&Margin {
                horizontal: 2,
                vertical: 2,
            });

            if area.area() > 0 {
                state.prepare_render(area);
                f.render_widget(&state, area);
            }

            prev_area = Some(area);
        })?;

        let event = crossterm::event::read()?;

        // skip duplicate event
        //   see https://github.com/crossterm-rs/crossterm/issues/772
        if let Event::Key(event) = event {
            if event.kind == KeyEventKind::Release {
                continue;
            }
        }

        if let Event::Key(KeyEvent {
            code: KeyCode::Char(code),
            modifiers,
            kind: _,
            state: _,
        }) = event
        {
            match code {
                'q' => break,
                'g' => {
                    state.gather_step(&mut requests);
                }
                'a' => {
                    // apply a single request
                    if let Some(request) = requests.pop_front() {
                        let eval = network.evaluate(&request.board);
                        zero_step_apply(&mut state.tree, request.respond(eval));
                    }
                }
                's' => {
                    // apply all outstanding requests
                    while let Some(request) = requests.pop_front() {
                        let eval = network.evaluate(&request.board);
                        zero_step_apply(&mut state.tree, request.respond(eval));
                    }

                    if modifiers.contains(KeyModifiers::ALT) {
                        // single batch
                        let start_visits = state.tree.root_visits();
                        state
                            .settings
                            .expand_tree(&mut state.tree, network, &mut state.rng, |tree| {
                                tree.root_visits() > start_visits
                            });
                    } else {
                        // single step
                        state.gather_step(&mut requests);
                        if let Some(request) = requests.pop_front() {
                            let eval = network.evaluate(&request.board);
                            zero_step_apply(&mut state.tree, request.respond(eval));
                        }
                        assert!(requests.is_empty());
                    }
                }

                _ => {}
            }
        }

        state.handle_event(prev_area.unwrap(), event);
    }

    // restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    Ok(())
}

const HEADER_SIZE: u16 = 2;
const OFFSET_MARGIN: usize = 3;
const COL_SPACING: u16 = 2;

#[derive(Debug, Clone)]
struct ColorString {
    string: String,
    color: Option<Color>,
}

impl ColorString {
    fn colored(string: String, color: Color) -> Self {
        ColorString {
            string,
            color: Some(color),
        }
    }
}

impl From<String> for ColorString {
    fn from(string: String) -> Self {
        ColorString { string, color: None }
    }
}

impl<B: Board> State<B> {
    fn node_board(&self, node: usize) -> B {
        if let Some(board) = self.board_cache.borrow().get(&node) {
            return board.clone();
        }

        let board = if let Some(parent) = self.tree[node].parent {
            let mut board = self.node_board(parent);
            board.play(self.tree[node].last_move.unwrap()).unwrap();
            board
        } else {
            self.tree.root_board().clone()
        };

        let prev = self.board_cache.borrow_mut().insert(node, board.clone());
        assert!(prev.is_none());
        board
    }

    fn gather_step(&mut self, requests: &mut VecDeque<ZeroRequest<B>>) {
        // gather a single node
        let request = zero_step_gather(
            &mut self.tree,
            self.settings.weights,
            self.settings.q_mode,
            self.settings.fpu_root,
            self.settings.fpu_child,
            self.settings.virtual_loss_weight,
            &mut self.rng,
        );
        if let Some(request) = request {
            requests.push_back(request)
        }
    }

    fn append_nodes(&self, curr: usize, depth: u32, result: &mut Vec<RenderNode>) {
        result.push(RenderNode { depth, node: curr });

        if self.expanded_nodes.contains(&curr) {
            if let Some(children) = self.tree[curr].children {
                let sorted_children = children
                    .iter()
                    .sorted_by_key(|&c| Reverse((self.tree[c].total_visits(), N32::from(self.tree[c].net_policy))));
                for c in sorted_children {
                    self.append_nodes(c, depth + 1, result);
                }
            }
        }
    }

    fn prepare_render(&mut self, area: Rect) {
        // collect nodes
        let mut nodes = std::mem::take(&mut self.prev_nodes);
        nodes.clear();
        self.append_nodes(0, 0, &mut nodes);
        self.prev_nodes = nodes;

        // fix offset
        let selected = self.selected_index();
        let margin = min(OFFSET_MARGIN, ((area.height - 1) / 2) as usize);
        let offset = (self.view_offset as i32).clamp(
            selected as i32 - (area.height as i32 - HEADER_SIZE as i32) + margin as i32,
            selected.saturating_sub(margin) as i32,
        );

        assert!(offset >= 0, "offset={}", offset);
        self.view_offset = offset as usize;
    }

    fn selected_index(&self) -> usize {
        self.prev_nodes
            .iter()
            .position(|n| n.node == self.selected_node)
            .unwrap()
    }

    fn handle_event(&mut self, area: Rect, event: Event) {
        match event {
            Event::Key(key) => match key.code {
                KeyCode::Up => {
                    let index = self.selected_index();
                    if index != 0 {
                        self.selected_node = self.prev_nodes[index - 1].node;
                    }
                }
                KeyCode::Down => {
                    self.selected_node = self
                        .prev_nodes
                        .get(self.selected_index() + 1)
                        .map_or(self.selected_node, |n| n.node);
                }
                KeyCode::Right => {
                    // try to expand the selected node
                    let expanded = self.expanded_nodes.insert(self.selected_node);

                    // if already expanded
                    if !expanded {
                        // expand and select first child
                        if let Some(child) = self.tree.best_child(self.selected_node) {
                            self.selected_node = child;
                            self.expanded_nodes.insert(child);
                        }
                    }
                }
                KeyCode::Left => {
                    // try to collapse the selected node
                    let collapsed = self.expanded_nodes.remove(&self.selected_node);

                    // if already collapsed
                    if !collapsed {
                        // collapse and select parent
                        if let Some(parent) = self.tree[self.selected_node].parent {
                            self.selected_node = parent;
                            self.expanded_nodes.remove(&parent);
                        }
                    }
                }
                _ => (),
            },
            Event::Mouse(mouse) => {
                if mouse.kind == MouseEventKind::Up(MouseButton::Left) {
                    let i = mouse.row as i32 + self.view_offset as i32 - area.y as i32 - HEADER_SIZE as i32;

                    if i >= 0 {
                        if let Some(node) = self.prev_nodes.get(i as usize) {
                            self.selected_node = node.node;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn compute_col_starts(&self, area: Rect) -> (Vec<u16>, Vec<u16>) {
        let mut col_sizes = vec![0; 1 + COLUMN_INFO.len()];
        col_sizes[0] = 20;

        for (i, (n1, n2, _, _)) in COLUMN_INFO.iter().enumerate() {
            col_sizes[i] = max(col_sizes[i], max(n1.len(), n2.len()) as u16);
        }

        for &RenderNode { node, depth } in &self.prev_nodes {
            for (i, v) in self.column_values(node, depth).iter().enumerate() {
                col_sizes[i] = max(col_sizes[i], v.string.len() as u16);
            }
        }

        let col_starts = col_sizes
            .iter()
            .scan(area.x, |curr, &size| {
                *curr += size + COL_SPACING;
                Some(*curr - size - COL_SPACING)
            })
            .collect_vec();

        (col_sizes, col_starts)
    }

    fn column_values(&self, node_index: usize, depth: u32) -> Vec<ColorString> {
        let board = self.node_board(node_index);
        let node = &self.tree[node_index];

        let arrow = if self.expanded_nodes.contains(&node_index) {
            "v"
        } else {
            ">"
        };

        let player = match board.next_player() {
            Player::A => ColorString::colored("A".to_owned(), Color::Green),
            Player::B => ColorString::colored("B".to_owned(), Color::Red),
        };

        let terminal = match node.outcome() {
            Err(_) => "?".to_owned().into(),
            Ok(None) => " ".to_owned().into(),
            Ok(Some(Outcome::WonBy(Player::A))) => ColorString::colored("A".to_owned(), Color::Green),
            Ok(Some(Outcome::Draw)) => ColorString::colored("D".to_owned(), Color::DarkGray),
            Ok(Some(Outcome::WonBy(Player::B))) => ColorString::colored("B".to_owned(), Color::Red),
        };

        let mut result: Vec<ColorString> = vec![];

        result.push(format!("{:>2$} {}", arrow, node_index, (depth * 2) as usize).into());
        result.push(player);
        result.push(format!("{}", display_option_empty(node.last_move)).into());
        result.push(terminal);

        if node.virtual_visits == 0 {
            result.push(format!("{}", node.complete_visits).into());
        } else {
            result.push(format!("{} +{}", node.complete_visits, node.virtual_visits).into());
        }

        // result.push(format!("{:?}", self.tree.depth_range(node_index)).into());
        // TODO depth calculations are slow for deep trees
        //   either cache, remove or speed up
        result.push(format!("{:?}", (0, 0)).into());

        {
            let zero = node.values();
            let net = node.net_values.unwrap_or(ZeroValuesAbs::nan());

            let (uct, zero_policy) = if let Some(parent_index) = node.parent {
                let uct_context = self.tree.uct_context(parent_index);
                let parent_board = self.node_board(parent_index);
                let parent = &self.tree[parent_index];

                let uct = node.uct(
                    uct_context,
                    self.settings.fpu_mode(parent_index == 0),
                    self.settings.q_mode,
                    self.settings.virtual_loss_weight,
                    parent_board.next_player(),
                );
                let zero_policy = node.complete_visits as f32 / (parent.complete_visits as f32 - 1.0).max(0.0);

                (uct, zero_policy)
            } else {
                (Uct::nan(), f32::NAN)
            };

            let values = [
                zero.value_abs.value_a,
                zero.wdl_abs.win_a,
                zero.wdl_abs.draw,
                zero.wdl_abs.win_b,
                zero.moves_left,
                zero_policy,
                net.value_abs.value_a,
                net.wdl_abs.win_a,
                net.wdl_abs.draw,
                net.wdl_abs.win_b,
                net.moves_left,
                node.net_policy,
                uct.q,
                uct.u,
                uct.m,
                uct.total(self.settings.weights),
            ];
            result.extend(
                values
                    .iter()
                    .map(|v| (if v.is_nan() { "".to_owned() } else { format!("{:.3}", v) }).into()),
            );
        }

        assert_eq!(result.len(), COLUMN_INFO.len());
        result
    }
}

const COLUMN_INFO: &[(&str, &str, bool, Color)] = &[
    ("Node", "", false, Color::Gray),
    ("Player", "", false, Color::Gray),
    ("Move", "", false, Color::Gray),
    ("T", "", false, Color::Gray),
    ("Visits", "", false, Color::Gray),
    ("Depth", "", false, Color::Gray),
    ("Zero", "V", true, Color::Magenta),
    ("Zero", "A", true, Color::Green),
    ("Zero", "D", true, Color::DarkGray),
    ("Zero", "B", true, Color::Red),
    ("Zero", "M", true, Color::Yellow),
    ("Zero", "P", true, Color::LightBlue),
    ("Net", "V", true, Color::Magenta),
    ("Net", "A", true, Color::Green),
    ("Net", "D", true, Color::DarkGray),
    ("Net", "B", true, Color::Red),
    ("Net", "M", true, Color::Yellow),
    ("Net", "P", true, Color::LightBlue),
    ("Uct", "Q", true, Color::Green),
    ("Uct", "U", true, Color::LightBlue),
    ("Uct", "M", true, Color::Yellow),
    ("Uct", "Total", true, Color::DarkGray),
];

impl<B: Board> Widget for &State<B> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (col_sizes, col_starts) = self.compute_col_starts(area);

        for (i, &(n1, n2, _, color)) in COLUMN_INFO.iter().enumerate() {
            if i == 0 || COLUMN_INFO[i - 1].0 != n1 {
                buf.set_string_safe(col_starts[i], area.y, n1, Style::default().fg(color));
            }
            buf.set_string_safe(col_starts[i], area.y + 1, n2, Style::default().fg(color));
        }

        for y in 0..area.height - HEADER_SIZE {
            let full_y = area.y + y + HEADER_SIZE;
            let i = y as u32 + self.view_offset as u32;

            if let Some(&RenderNode { node, depth }) = self.prev_nodes.get(i as usize) {
                if node == self.selected_node {
                    let line = Rect::new(area.x, full_y, area.width, 1);
                    let style = Style::default().add_modifier(Modifier::REVERSED);
                    buf.set_style(line, style);
                }

                for (i, v) in self.column_values(node, depth).iter().enumerate() {
                    let just_right = COLUMN_INFO[i].2;
                    let column_color = COLUMN_INFO[i].3;
                    let color = v.color.unwrap_or(column_color);

                    let x = if just_right {
                        col_starts[i] + (col_sizes[i] - v.string.len() as u16)
                    } else {
                        col_starts[i]
                    };

                    buf.set_string_safe(x, full_y, &v.string, Style::default().fg(color));
                }
            }
        }
    }
}

trait BufExt {
    fn set_string_safe(&mut self, x: u16, y: u16, s: impl AsRef<str>, style: Style);
}

impl BufExt for Buffer {
    fn set_string_safe(&mut self, x: u16, y: u16, s: impl AsRef<str>, style: Style) {
        let has_x = (self.area.left()..self.area.right()).contains(&x);
        let has_y = (self.area.top()..self.area.bottom()).contains(&y);

        if has_x && has_y {
            self.set_stringn(x, y, s, usize::MAX, style);
        }
    }
}
