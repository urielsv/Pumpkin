use std::{net::IpAddr, str::FromStr};

use crate::{
    command::{
        args::{simple::SimpleArgConsumer, Arg, ConsumedArgs},
        tree::CommandTree,
        tree_builder::argument,
        CommandError, CommandExecutor, CommandSender,
    },
    data::{banned_ip_data::BANNED_IP_LIST, SaveJSONConfiguration},
};
use async_trait::async_trait;
use pumpkin_util::text::TextComponent;
use CommandError::InvalidConsumption;

const NAMES: [&str; 1] = ["pardon-ip"];
const DESCRIPTION: &str = "unbans a ip";

const ARG_TARGET: &str = "ip";

struct PardonIpExecutor;

#[async_trait]
impl CommandExecutor for PardonIpExecutor {
    async fn execute<'a>(
        &self,
        sender: &mut CommandSender<'a>,
        _server: &crate::server::Server,
        args: &ConsumedArgs<'a>,
    ) -> Result<(), CommandError> {
        let Some(Arg::Simple(target)) = args.get(&ARG_TARGET) else {
            return Err(InvalidConsumption(Some(ARG_TARGET.into())));
        };

        let Ok(ip) = IpAddr::from_str(target) else {
            sender
                .send_message(TextComponent::translate(
                    "commands.pardonip.invalid",
                    [].into(),
                ))
                .await;
            return Ok(());
        };

        let mut lock = BANNED_IP_LIST.write().await;

        if let Some(idx) = lock.banned_ips.iter().position(|entry| entry.ip == ip) {
            lock.banned_ips.remove(idx);
        } else {
            sender
                .send_message(TextComponent::translate(
                    "commands.pardonip.failed",
                    [].into(),
                ))
                .await;
            return Ok(());
        }

        lock.save();

        sender
            .send_message(TextComponent::translate(
                "commands.pardonip.success",
                [TextComponent::text(ip.to_string())].into(),
            ))
            .await;
        Ok(())
    }
}

pub fn init_command_tree() -> CommandTree {
    CommandTree::new(NAMES, DESCRIPTION)
        .then(argument(ARG_TARGET, SimpleArgConsumer).execute(PardonIpExecutor))
}
