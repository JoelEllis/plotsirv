use valence::client::despawn_disconnected_clients;
use valence::client::event::{
    default_event_handler, FinishDigging, StartDigging, StartSneaking, UseItemOnBlock,
};
use valence::prelude::*;
use valence_protocol::types::Hand;

const SPAWN_Y: i32 = 64;

pub fn main() {
    tracing_subscriber::fmt().init();

    App::new()
        .add_plugin(ServerPlugin::new(()))
        .add_system_to_stage(EventLoop, default_event_handler)
        .add_system_to_stage(EventLoop, toggle_gamemode_on_sneak)
        .add_system_to_stage(EventLoop, digging_creative_mode)
        .add_system_to_stage(EventLoop, digging_survival_mode)
        .add_system_to_stage(EventLoop, place_blocks)
        .add_system_set(PlayerList::default_system_set())
        .add_startup_system(setup)
        .add_system(init_clients)
        .add_system(despawn_disconnected_clients)
        .run();
}

fn setup(world: &mut World) {
    let mut instance = world
        .resource::<Server>()
        .new_instance(DimensionId::default());

    for z in -5..5 {
        for x in -5..5 {
            instance.insert_chunk([x, z], Chunk::default());
        }
    }

    for z in -25..25 {
        for x in -25..25 {
            instance.set_block_state([x, SPAWN_Y, z], BlockState::GRASS_BLOCK);
        }
    }

    world.spawn(instance);
}

fn init_clients(
    mut clients: Query<&mut Client, Added<Client>>,
    instances: Query<Entity, With<Instance>>,
) {
    for mut client in &mut clients {
        client.set_position([0.0, SPAWN_Y as f64 + 1.0, 0.0]);
        client.set_instance(instances.single());
        client.set_game_mode(GameMode::Creative);
        client.send_message("Welcome to Valence! Build something cool.".italic());
    }
}

fn toggle_gamemode_on_sneak(
    mut clients: Query<&mut Client>,
    mut events: EventReader<StartSneaking>,
) {
    for event in events.iter() {
        let Ok(mut client) = clients.get_component_mut::<Client>(event.client) else {
            continue;
        };
        let mode = client.game_mode();
        client.set_game_mode(match mode {
            GameMode::Survival => GameMode::Creative,
            GameMode::Creative => GameMode::Survival,
            _ => GameMode::Creative,
        });
    }
}

fn digging_creative_mode(
    clients: Query<&Client>,
    mut instances: Query<&mut Instance>,
    mut events: EventReader<StartDigging>,
) {
    let mut instance = instances.single_mut();

    for event in events.iter() {
        let Ok(client) = clients.get_component::<Client>(event.client) else {
            continue;
        };
        if client.game_mode() == GameMode::Creative {
            instance.set_block_state(event.position, BlockState::AIR);
        }
    }
}

fn digging_survival_mode(
    clients: Query<&Client>,
    mut instances: Query<&mut Instance>,
    mut events: EventReader<FinishDigging>,
) {
    let mut instance = instances.single_mut();

    for event in events.iter() {
        let Ok(client) = clients.get_component::<Client>(event.client) else {
            continue;
        };
        if client.game_mode() == GameMode::Survival {
            instance.set_block_state(event.position, BlockState::AIR);
        }
    }
}

fn place_blocks(
    mut clients: Query<(&Client, &mut Inventory)>,
    mut instances: Query<&mut Instance>,
    mut events: EventReader<UseItemOnBlock>,
) {
    let mut instance = instances.single_mut();

    for event in events.iter() {
        let Ok((client, mut inventory)) = clients.get_mut(event.client) else {
            continue;
        };
        if event.hand != Hand::Main {
            continue;
        }

        // get the held item
        let slot_id = client.held_item_slot();
        let Some(stack) = inventory.slot(slot_id) else {
            // no item in the slot
            continue;
        };

        let Some(block_kind) = stack.item.to_block_kind() else {
            // can't place this item as a block
            continue;
        };

        if client.game_mode() == GameMode::Survival {
            // check if the player has the item in their inventory and remove
            // it.
            let slot = if stack.count() > 1 {
                let mut stack = stack.clone();
                stack.set_count(stack.count() - 1);
                Some(stack)
            } else {
                None
            };
            inventory.replace_slot(slot_id, slot);
        }

        // TODO: client.facing()?
        let facing = match client.yaw().rem_euclid(360.0) {
            yaw if (yaw >= 315.0) || (yaw < 45.0) => PropValue::South,
            yaw if (yaw >= 45.0) && (yaw < 135.0) => PropValue::West,
            yaw if (yaw >= 135.0) && (yaw < 225.0) => PropValue::North,
            yaw if (yaw >= 225.0) && (yaw < 315.0) => PropValue::East,

            _ => unreachable!(),
        };

        let mut block_state = block_kind.to_state();

        let replace = instance.block_state(event.position).is_replaceable();

        // TODO: Is there a better way to do this?
        // - a has_prop api?
        // - a is_stairs, is_slab, etc api?
        let has_facing = block_state.get(PropName::Facing).is_some();
        let has_half = block_state.get(PropName::Half).is_some();

        let has_type = block_state.get(PropName::Type).is_some();

        if has_facing {
            block_state = block_state.set(PropName::Facing, facing);
        }

        if has_half || has_type {
            match event.face {
                valence_protocol::BlockFace::Bottom => {
                    block_state = block_state
                        .set(PropName::Half, PropValue::Top)
                        .set(PropName::Type, PropValue::Top);
                }
                valence_protocol::BlockFace::Top => {
                    block_state = block_state
                        .set(PropName::Half, PropValue::Bottom)
                        .set(PropName::Type, PropValue::Bottom);
                }
                valence_protocol::BlockFace::North
                | valence_protocol::BlockFace::South
                | valence_protocol::BlockFace::West
                | valence_protocol::BlockFace::East => {
                    let top = event.cursor_pos[1] > 0.5;
                    let val = match top {
                        true => PropValue::Top,
                        false => PropValue::Bottom,
                    };
                    block_state = block_state
                        .set(PropName::Half, val)
                        .set(PropName::Type, val);
                }
            }
        }

        // !TODO:
        // - Combine slabs
        // - 2-high doors
        // - Open/close (trap)doors
        // - Stair bending

        let real_pos = if replace {
            event.position
        } else {
            event.position.get_in_direction(event.face)
        };
        instance.set_block_state(real_pos, block_state);
    }
}
