// we're going to be doing a lot of unsafe stuff so yeah
#![deny(unsafe_op_in_unsafe_fn)]

use std::io::{Seek,SeekFrom};
use std::mem::size_of;

use num_derive::FromPrimitive;
use byteorder::{LittleEndian, ReadBytesExt};

use crate::hook_helpers::*;
use crate::game_data::*;
use crate::storage;
use crate::storage::RegisterType;
use crate::character_extensions;
use crate::var_rw;

/// This is the number after the 0x66
/// All commands that start with 0x66 should be ones added by me (anotak). if you want to add commands you should reserve another starting value to prevent conflicts. (game uses commands 0 through 7 inclusive)
#[derive(FromPrimitive)]
#[repr(u32)]
pub enum AnoCmd
{
    RelativeTeleportX = 0x00,
    RelativeTeleportY = 0x01,
    
    LoadImmediateIntoRegister = 0x10,
    BinaryOperationRegisterImmediate = 0x11,
    BinaryOperationRegisterRegister = 0x12,
    UnaryOperationRegister = 0x13,
    UnaryOperationImmediate = 0x14,
    LoadVarIntoRegister = 0x15,
    StoreVarFromRegister = 0x16,
    LoadOpponentVarIntoRegister = 0x17,
    StoreOpponentVarFromRegister = 0x18,
    
    SuckX = 0x50,
}

/// handle commands starting in 66
pub fn handle_ano_command(command : AnoCmd, exe_char : Char, command_ptr : usize)
{
    match command {
        AnoCmd::RelativeTeleportX => {
            //debug_msg(format!("exe_char = {}\np1c1 = {:#X}\np1c2 = {:#X}\np1c3 = {:#X}\np2c1 = {:#X}\np2c2 = {:#X}\np2c3 = {:#X}", exe_char, get_p1_char1_ptr(), get_p1_char2_ptr(), get_p1_char3_ptr(), get_p2_char1_ptr(), get_p2_char2_ptr(), get_p2_char3_ptr()));
            
            let my_team = exe_char.identify_team();
            //debug_msg(format!("my_team = {:?}", my_team));
            let op_team = my_team.opposite();
            //debug_msg(format!("op_team = {:?}", op_team));
            
            let offset : f32 = { 
                let offset = unsafe { read_ptr_no_check::<f32>(command_ptr) };
                
                if offset.is_finite() {
                    offset
                } else {
                    0.0
                }
            };
            
            
            // we put this into a local function in order to not duplicate p1/p2 code
            let get_destination = |exe_char : &Char, op_char  : &Char| -> f32
            {
                let op_pos = op_char.get_x_pos();
                let facing = exe_char.get_facing();
                
                let offset = if facing == Facing::Right { -offset } else { offset };
                
                op_pos + offset
            };
            
            
            let x_pos = match op_team {
                Team::Player1 => get_destination(&exe_char, &Char::get_p1_point()),
                Team::Player2 => get_destination(&exe_char, &Char::get_p2_point()),
                Team::Unknown => {
                    let p1 = Char::get_p1_point();
                    let p2 = Char::get_p2_point();
                    
                    // this is basically an error. if we can't identify which team this character
                    // belongs to, then we just teleport them into the average between the two
                    // point chars since that seems like the safest and fairest choice
                    // other options might be: don't move the character?
                    (p1.get_x_pos() + p2.get_x_pos()) * 0.5
                },
            };
        
            exe_char.set_x_pos(x_pos);
        },
        AnoCmd::RelativeTeleportY => {
            let my_team = exe_char.identify_team();
            let op_team = my_team.opposite();
            
            let offset : f32 = { 
                let offset = unsafe { read_ptr_no_check::<f32>(command_ptr) };
                
                if offset.is_finite() {
                    offset
                } else {
                    0.0
                }
            };
            
            
            
            // we put this into a local function in order to not duplicate p1/p2 code
            let get_destination = |op_char  : &Char| -> f32
            {
                let op_pos = op_char.get_y_pos();
                let hitstun_state = op_char.get_hitstun_non_knockdown();
                
                // TODO - investigate more and see if this holds true all the time
                if op_pos > 0.0 && hitstun_state == HitstunFlagA::HitstunAirStandCrouch {
                    // seems like enemies in juggleable states are offset by 96
                    op_pos + offset - 96.0
                } else {
                    op_pos + offset
                }
            };
            
            
            let y_pos = match op_team {
                Team::Player1 => get_destination(&Char::get_p1_point()),
                Team::Player2 => get_destination(&Char::get_p2_point()),
                Team::Unknown => {
                    let p1 = Char::get_p1_point();
                    let p2 = Char::get_p2_point();
                    
                    // this is basically an error. if we can't identify which team this character
                    // belongs to, then we just teleport them into the average between the two
                    // point chars since that seems like the safest and fairest choice
                    // other options might be: don't move the character?
                    (p1.get_y_pos() + p2.get_y_pos()) * 0.5 + offset
                },
            };
        
            exe_char.set_y_pos(y_pos);
        },
        AnoCmd::LoadImmediateIntoRegister => {
            let mut cursor = unsafe { get_cursor(command_ptr, const { size_of::<u32>() * 2 }) };
            
            cursor.seek(SeekFrom::Current(3)).unwrap();
            let destination = cursor.read_u8().unwrap();
            
            storage::with(
                exe_char.get_ptr(),
                |store| {
                    store.read_into_register(destination, &mut cursor)
                }
            );
        },
        AnoCmd::BinaryOperationRegisterImmediate => {
            let mut cursor = unsafe { get_cursor(command_ptr, const { size_of::<u32>() * 3 }) };
            
            let operation = cursor.read_u32::<LittleEndian>().unwrap();
            let operation = num::FromPrimitive::from_u32(operation);
            
            let lhs = cursor.read_u8().unwrap();
            cursor.seek(SeekFrom::Current(2)).unwrap();
            let destination = cursor.read_u8().unwrap();
            
            let op_type = RegisterType::identify(destination);
            
            //debug_msg(format!("reg imm operation = {:?} destination = {:?}", operation, destination));
            
            if let Some(operation) = operation {
                storage::with(
                    exe_char.get_ptr(),
                    |store| {
                        match op_type {
                            RegisterType::F32 => {
                                let rhs = cursor.read_f32::<LittleEndian>().unwrap();
                                
                                store.register_imm_operation_f32(lhs, rhs, destination, operation);
                            },
                            RegisterType::I32 => {
                                let rhs = cursor.read_i32::<LittleEndian>().unwrap();
                                
                                store.register_imm_operation_i32(lhs, rhs, destination, operation);
                            },
                        };
                    }
                );
            }
        },
        AnoCmd::BinaryOperationRegisterRegister => {
            let mut cursor = unsafe { get_cursor(command_ptr, const { size_of::<u32>() * 2 }) };
            
            let operation = cursor.read_u32::<LittleEndian>().unwrap();
            let operation = num::FromPrimitive::from_u32(operation);
            
            let lhs = cursor.read_u8().unwrap();
            let rhs = cursor.read_u8().unwrap();
            cursor.seek(SeekFrom::Current(1)).unwrap();
            let destination = cursor.read_u8().unwrap();
            
            if let Some(operation) = operation {
                storage::with(
                    exe_char.get_ptr(),
                    |store| {
                        store.register_register_operation(lhs, rhs, destination, operation);
                    }
                );
            }
        },
        AnoCmd::UnaryOperationRegister => {
            
            let mut cursor = unsafe { get_cursor(command_ptr, const { size_of::<u32>() * 2 }) };
            
            let operation = cursor.read_u32::<LittleEndian>().unwrap();
            let operation = num::FromPrimitive::from_u32(operation);
            
            let reg = cursor.read_u8().unwrap();
            cursor.seek(SeekFrom::Current(2)).unwrap();
            let destination = cursor.read_u8().unwrap();
            
            if let Some(operation) = operation {
                storage::with(
                    exe_char.get_ptr(),
                    |store| {
                        store.register_unary_operation(reg, destination, operation);
                    }
                );
            }
        }, 
        AnoCmd::UnaryOperationImmediate => {
            let mut cursor = unsafe { get_cursor(command_ptr, const { size_of::<u32>() * 3 }) };
            
            let operation = cursor.read_u32::<LittleEndian>().unwrap();
            let operation = num::FromPrimitive::from_u32(operation);
            
            cursor.seek(SeekFrom::Current(3)).unwrap();
            let destination = cursor.read_u8().unwrap();
            
            let op_type = RegisterType::identify(destination);
            
            if let Some(operation) = operation {
                storage::with(
                    exe_char.get_ptr(),
                    |store| {
                        match op_type {
                            RegisterType::F32 => {
                                let immediate = cursor.read_f32::<LittleEndian>().unwrap();
                                
                                store.immediate_unary_operation_f32(immediate, destination, operation);
                            },
                            RegisterType::I32 => {
                                let immediate = cursor.read_i32::<LittleEndian>().unwrap();
                                
                                store.immediate_unary_operation_i32(immediate, destination, operation);
                            },
                        };
                    }
                );
            }
        }, 
        AnoCmd::LoadVarIntoRegister => {
            load_var_into_register(exe_char, command_ptr)
        }, 
        AnoCmd::StoreVarFromRegister => {
            store_var_into_register(exe_char, command_ptr)
        },
        AnoCmd::LoadOpponentVarIntoRegister => {
            if let Some(opponent) = exe_char.get_opponent_point_char() {
                load_var_into_register(opponent, command_ptr)
            }
        }, 
        AnoCmd::StoreOpponentVarFromRegister => {
            if let Some(opponent) = exe_char.get_opponent_point_char() {
                store_var_into_register(opponent, command_ptr)
            }
        },
        AnoCmd::SuckX => {
            use character_extensions::SuckOpponent;
            
            let mut cursor = unsafe { get_cursor(command_ptr, const { size_of::<f32>() * 2 }) };
            let magnitude = cursor.read_f32::<LittleEndian>().unwrap();
            let delta = cursor.read_f32::<LittleEndian>().unwrap();
            
            SuckOpponent::apply_suck(exe_char, magnitude, delta);
        },
    }
}


fn load_var_into_register(character : Char, command_ptr : usize)
{
    let mut cursor = unsafe { get_cursor(command_ptr, const { size_of::<u32>() * 2 }) };
    
    cursor.seek(SeekFrom::Current(3)).unwrap();
    let destination = cursor.read_u8().unwrap();
    
    let destination_type = RegisterType::identify(destination);
    
    let var = cursor.read_u32::<LittleEndian>().unwrap();
    
    storage::with(
        character.get_ptr(),
        |store| {
            match destination_type {
                RegisterType::F32 => {
                    let result = var_rw::MatchState::load_f32(character.get_ptr(), var);
                    
                    store.set_f32_register(destination, result);
                    
                    character.set_condition_register(result as i32);
                },
                RegisterType::I32 => {
                    let result = var_rw::MatchState::load_i32(character.get_ptr(), var);
                    
                    store.set_i32_register(destination, result);
                    
                    character.set_condition_register(result);
                },
            };
        }
    );
}

fn store_var_into_register(character : Char, command_ptr : usize)
{
    let mut cursor = unsafe { get_cursor(command_ptr, const { size_of::<u32>() * 2 }) };
    
    cursor.seek(SeekFrom::Current(3)).unwrap();
    let source = cursor.read_u8().unwrap();
    
    let source_type = RegisterType::identify(source);
    let var = cursor.read_u32::<LittleEndian>().unwrap();
    
    storage::with(
        character.get_ptr(),
        |store| {
            match source_type {
                RegisterType::F32 => {
                    let source_value = store.get_f32_register(source);
                    
                    var_rw::MatchState::store_f32(character.get_ptr(), var, source_value);
                    
                    character.set_condition_register(source_value as i32);
                },
                RegisterType::I32 => {
                    let source_value = store.get_i32_register(source);
                    
                    var_rw::MatchState::store_i32(character.get_ptr(), var, source_value);
                    
                    character.set_condition_register(source_value);
                },
            };
        }
    );
}

