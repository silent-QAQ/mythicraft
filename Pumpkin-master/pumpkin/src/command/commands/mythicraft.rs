use crate::command::args::position_3d::Position3DArgumentConsumer;
use crate::command::args::simple::SimpleArgConsumer;
use crate::command::args::{Arg, ConsumedArgs, FindArg};
use crate::command::tree::builder::{argument, literal};
use crate::command::{CommandError, CommandExecutor, CommandResult, CommandSender};
use crate::mythicraft::MythicraftCoreError;
use crate::server::Server;
use mythicraft_rpg::runtime::Position as RpgPosition;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionLvl, PermissionRegistry};
use pumpkin_util::text::TextComponent;

const DESCRIPTION: &str = "Controls the native Mythicraft RPG runtime.";
const PERMISSION: &str = "mythicraft:command.rpg";
const ARG_DEFINITION: &str = "definition";
const ARG_POSITION: &str = "pos";
const ARG_SOURCE: &str = "source";
const ARG_SKILL: &str = "skill";

struct StatusExecutor;
struct SpawnExecutor;
struct SkillExecutor;
struct UiExecutor;

fn command_failure(error: MythicraftCoreError) -> CommandError {
    CommandError::CommandFailed(TextComponent::text(format!("Mythicraft: {error}")))
}

fn spawn_context(
    sender: &CommandSender,
    server: &Server,
    args: &ConsumedArgs<'_>,
) -> Result<(std::sync::Arc<crate::world::World>, Vector3<f64>), CommandError> {
    let explicit_position = match args.get(ARG_POSITION) {
        Some(Arg::Pos3D(position)) => Some(*position),
        _ => None,
    };
    match sender {
        CommandSender::Player(player) => Ok((
            player.world(),
            explicit_position.unwrap_or_else(|| player.position()),
        )),
        CommandSender::CommandBlock(block, world) => Ok((
            world.clone(),
            explicit_position.unwrap_or_else(|| block.get_position().to_centered_f64()),
        )),
        CommandSender::Console | CommandSender::Rcon(_) | CommandSender::Dummy => {
            let world = server
                .worlds
                .load()
                .first()
                .cloned()
                .ok_or(CommandError::InvalidRequirement)?;
            let position = explicit_position.unwrap_or_else(|| {
                let info = world.level_info.load();
                Vector3::new(
                    f64::from(info.spawn_x) + 0.5,
                    f64::from(info.spawn_y) + 1.0,
                    f64::from(info.spawn_z) + 0.5,
                )
            });
            Ok((world, position))
        }
    }
}

impl CommandExecutor for StatusExecutor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        server: &'a Server,
        _args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let rpg = server.mythicraft.rpg.lock().await;
            sender
                .send_message(TextComponent::text(format!(
                    "[Mythicraft] definitions={}, skills={}, runtime_entities={}, tick={}",
                    rpg.document.entities.len(),
                    rpg.document
                        .entities
                        .iter()
                        .map(|definition| definition.skills.len())
                        .sum::<usize>(),
                    rpg.entities.len(),
                    rpg.tick,
                )))
                .await;
            Ok(1)
        })
    }
}

impl CommandExecutor for SpawnExecutor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        server: &'a Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let definition = SimpleArgConsumer::find_arg(args, ARG_DEFINITION)?;
            let (world, position) = spawn_context(sender, server, args)?;
            let world_name = world.get_world_name().to_owned();
            match server
                .mythicraft
                .spawn_definition(
                    server,
                    Some(&world_name),
                    definition,
                    RpgPosition {
                        x: position.x,
                        y: position.y,
                        z: position.z,
                    },
                )
                .await
            {
                Ok(runtime_id) => {
                    sender
                        .send_message(TextComponent::text(format!(
                            "[Mythicraft] spawned {definition} as {runtime_id}"
                        )))
                        .await;
                    Ok(1)
                }
                Err(error) => Err(command_failure(error)),
            }
        })
    }
}

impl CommandExecutor for SkillExecutor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        server: &'a Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let source = SimpleArgConsumer::find_arg(args, ARG_SOURCE)?;
            let skill = SimpleArgConsumer::find_arg(args, ARG_SKILL)?;
            match server
                .mythicraft
                .execute_skill(server, source, skill, None)
                .await
            {
                Ok(events) => {
                    sender
                        .send_message(TextComponent::text(format!(
                            "[Mythicraft] skill {skill} from {source}: {events} event(s)"
                        )))
                        .await;
                    Ok(events as i32)
                }
                Err(error) => Err(command_failure(error)),
            }
        })
    }
}

impl CommandExecutor for UiExecutor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        server: &'a Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let page_id = SimpleArgConsumer::find_arg(args, "page")?;
            let CommandSender::Player(player) = sender else {
                return Err(CommandError::InvalidRequirement);
            };
            if server.mythicraft.open_arcartx_page(player, page_id).await {
                sender
                    .send_message(TextComponent::text(format!(
                        "[Mythicraft] opened ArcartX page {page_id}"
                    )))
                    .await;
                Ok(1)
            } else {
                Err(CommandError::CommandFailed(TextComponent::text(format!(
                    "Mythicraft: ArcartX page not found or client capability is unavailable: {page_id}"
                ))))
            }
        })
    }
}

pub fn register(
    dispatcher: &mut crate::command::dispatcher::CommandDispatcher,
    registry: &mut PermissionRegistry,
) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));
    dispatcher.register(
        crate::command::tree::CommandTree::new(["mythicraft", "mc-rpg"], DESCRIPTION)
            .execute(StatusExecutor)
            .then(
                literal("spawn").then(
                    argument(ARG_DEFINITION, SimpleArgConsumer)
                        .execute(SpawnExecutor)
                        .then(
                            argument(ARG_POSITION, Position3DArgumentConsumer)
                                .execute(SpawnExecutor),
                        ),
                ),
            )
            .then(
                literal("skill").then(
                    argument(ARG_SOURCE, SimpleArgConsumer)
                        .then(argument(ARG_SKILL, SimpleArgConsumer).execute(SkillExecutor)),
                ),
            )
            .then(literal("ui").then(argument("page", SimpleArgConsumer).execute(UiExecutor))),
        PERMISSION,
    );
}
