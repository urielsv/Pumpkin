use async_trait::async_trait;
use pumpkin_data::world::{MSG_COMMAND_INCOMING, MSG_COMMAND_OUTGOING};
use pumpkin_util::text::{click::ClickEvent, hover::HoverEvent, TextComponent};

use crate::command::{
    args::{
        message::MsgArgConsumer, players::PlayersArgumentConsumer, Arg, ConsumedArgs,
        FindArgDefaultName,
    },
    tree::CommandTree,
    tree_builder::{argument, argument_default_name},
    CommandError, CommandExecutor, CommandSender,
};
use CommandError::InvalidConsumption;

const NAMES: [&str; 3] = ["msg", "tell", "w"];

const DESCRIPTION: &str = "Sends a private message to one or more players.";

const ARG_MESSAGE: &str = "message";

struct MsgExecutor;

#[async_trait]
impl CommandExecutor for MsgExecutor {
    async fn execute<'a>(
        &self,
        sender: &mut CommandSender<'a>,
        _server: &crate::server::Server,
        args: &ConsumedArgs<'a>,
    ) -> Result<(), CommandError> {
        let Some(Arg::Msg(msg)) = args.get(ARG_MESSAGE) else {
            return Err(InvalidConsumption(Some(ARG_MESSAGE.into())));
        };
        let targets = PlayersArgumentConsumer.find_arg_default_name(args)?;
        let player = sender.as_player().ok_or(CommandError::InvalidRequirement)?;

        for target in targets {
            player
                .send_message(
                    &TextComponent::text(msg.clone()),
                    MSG_COMMAND_OUTGOING,
                    &TextComponent::text(player.gameprofile.name.clone()),
                    Some(
                        &TextComponent::text(target.gameprofile.name.clone())
                            .hover_event(HoverEvent::show_entity(
                                target.living_entity.entity.entity_uuid.to_string(),
                                Some(
                                    format!("{:?}", target.living_entity.entity.entity_type)
                                        .to_lowercase(),
                                ),
                                Some(TextComponent::text(target.gameprofile.name.clone())),
                            ))
                            .click_event(ClickEvent::SuggestCommand(
                                format!("/tell {} ", target.gameprofile.name.clone()).into(),
                            )),
                    ),
                )
                .await;
        }
        for target in targets {
            target
                .send_message(
                    &TextComponent::text(msg.clone()),
                    MSG_COMMAND_INCOMING,
                    &TextComponent::text(player.gameprofile.name.clone())
                        .hover_event(HoverEvent::show_entity(
                            player.living_entity.entity.entity_uuid.to_string(),
                            Some(
                                format!("{:?}", player.living_entity.entity.entity_type)
                                    .to_lowercase(),
                            ),
                            Some(TextComponent::text(player.gameprofile.name.clone())),
                        ))
                        .click_event(ClickEvent::SuggestCommand(
                            format!("/tell {} ", player.gameprofile.name.clone()).into(),
                        )),
                    Some(&TextComponent::text(target.gameprofile.name.clone())),
                )
                .await;
        }

        Ok(())
    }
}

pub fn init_command_tree() -> CommandTree {
    CommandTree::new(NAMES, DESCRIPTION).then(
        argument_default_name(PlayersArgumentConsumer)
            .then(argument(ARG_MESSAGE, MsgArgConsumer).execute(MsgExecutor)),
    )
}
