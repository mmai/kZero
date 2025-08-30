use board_game::board::{Board, Player};
use trictrac_bot::training_common::{TrictracAction, ACTION_SPACE_SIZE};
use trictrac_bot::trictrac_board::TrictracBoard;
use trictrac_store::Color;
use trictrac_store::TurnStage;

use crate::mapping::bit_buffer::BitBuffer;
use crate::mapping::{InputMapper, MuZeroMapper, PolicyMapper};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct TrictracStdMapper;

impl InputMapper<TrictracBoard> for TrictracStdMapper {
    fn input_bool_shape(&self) -> [usize; 3] {
        // 15 checkers for each player
        // 24 places
        [2 * 15, 24, 1]
    }

    fn input_scalar_count(&self) -> usize {
        // turn stage
        // black_turn, white_turn
        // 2 dice
        // For each player: points, holes, can_bredouille, can_big_bredouille, dice_roll_count
        1 + 2 + 2 + 2 * 5
    }

    fn encode_input(&self, bools: &mut BitBuffer, scalars: &mut Vec<f32>, board: &TrictracBoard) {
        let game_state = board.inner();
        let pov_player = game_state.who_plays().unwrap();
        let opp_player = game_state
            .players
            .values()
            .find(|p| p.color != pov_player.color)
            .unwrap();

        // Encode board state from POV player
        let positions = game_state.board.to_vec();
        for player in [pov_player, opp_player] {
            for nb_checkers in 0..15 {
                for field in 1..25 {
                    let (count, color) = game_state.board.get_field_checkers(field).unwrap();
                    bools.push(Some(&player.color) == color && nb_checkers == count);
                }
            }
        }

        // ---- Scalars
        // Turn stage
        scalars.push(game_state.turn_stage as u8 as f32);

        // Active player
        for color in [Color::White, Color::Black] {
            scalars.push((pov_player.color == color) as u8 as f32);
        }

        // Dice
        scalars.push(game_state.dice.values.0 as f32 / 6.0);
        scalars.push(game_state.dice.values.1 as f32 / 6.0);

        // Player stats
        for player in [pov_player, opp_player] {
            scalars.push(player.points as f32);
            scalars.push(player.holes as f32);
            scalars.push(player.can_bredouille as u8 as f32);
            scalars.push(player.can_big_bredouille as u8 as f32);
            scalars.push(player.dice_roll_count as f32);
        }
    }
}

impl PolicyMapper<TrictracBoard> for TrictracStdMapper {
    fn policy_shape(&self) -> &[usize] {
        &[ACTION_SPACE_SIZE]
    }

    fn move_to_index(&self, _board: &TrictracBoard, mv: TrictracAction) -> usize {
        TrictracAction::from(mv).to_action_index()
    }

    fn index_to_move(&self, board: &TrictracBoard, index: usize) -> Option<TrictracAction> {
        TrictracAction::from_action_index(index)
    }
}

impl MuZeroMapper<TrictracBoard> for TrictracStdMapper {
    fn state_board_size(&self) -> usize {
        panic!("not implemented");
    }

    fn encoded_move_shape(&self) -> [usize; 3] {
        panic!("not implemented");
    }

    fn encode_mv(&self, result: &mut Vec<f32>, move_index: usize) {
        panic!("not implemented");
    }
}
