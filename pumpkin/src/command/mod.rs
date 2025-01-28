use std::fmt;
use std::sync::Arc;

use crate::command::commands::seed;
use crate::command::commands::{bossbar, transfer};
use crate::command::dispatcher::CommandDispatcher;
use crate::entity::player::Player;
use crate::server::Server;
use crate::world::World;
use args::ConsumedArgs;
use async_trait::async_trait;
use commands::{
    ban, banip, banlist, clear, deop, fill, gamemode, give, help, kick, kill, list, me, msg, op,
    pardon, pardonip, playsound, plugin, plugins, pumpkin, say, setblock, stop, summon, teleport,
    time, title, worldborder,
};
use dispatcher::CommandError;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::permission::PermissionLvl;
use pumpkin_util::text::TextComponent;

pub mod args;
pub mod client_cmd_suggestions;
mod commands;
pub mod dispatcher;
pub mod tree;
pub mod tree_builder;
mod tree_format;

pub enum CommandSender<'a> {
    Rcon(&'a tokio::sync::Mutex<Vec<String>>),
    Console,
    Player(Arc<Player>),
}

impl fmt::Display for CommandSender<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                CommandSender::Console => "Server",
                CommandSender::Rcon(_) => "Rcon",
                CommandSender::Player(p) => &p.gameprofile.name,
            }
        )
    }
}

impl CommandSender<'_> {
    pub async fn send_message(&self, text: TextComponent) {
        match self {
            CommandSender::Console => log::info!("{}", text.to_pretty_console()),
            CommandSender::Player(c) => c.send_system_message(&text).await,
            CommandSender::Rcon(s) => s.lock().await.push(text.to_pretty_console()),
        }
    }

    pub const fn is_player(&self) -> bool {
        matches!(self, CommandSender::Player(_))
    }

    pub const fn is_console(&self) -> bool {
        matches!(self, CommandSender::Console)
    }

    pub fn as_player(&self) -> Option<Arc<Player>> {
        match self {
            CommandSender::Player(player) => Some(player.clone()),
            _ => None,
        }
    }

    pub fn permission_lvl(&self) -> PermissionLvl {
        match self {
            CommandSender::Console | CommandSender::Rcon(_) => PermissionLvl::Four,
            CommandSender::Player(p) => p.permission_lvl.load(),
        }
    }

    pub fn has_permission_lvl(&self, lvl: PermissionLvl) -> bool {
        match self {
            CommandSender::Console | CommandSender::Rcon(_) => true,
            CommandSender::Player(p) => p.permission_lvl.load().ge(&lvl),
        }
    }

    pub fn has_permission(&self, permission: &str) -> bool {
        match self {
            CommandSender::Console => true, // Console always has permission
            CommandSender::Rcon(_) => true, // RCON always has permission
            CommandSender::Player(player) => {
                // For core minecraft/pumpkin commands, require both permission level and permission node
                if permission.starts_with("minecraft.command.") {
                    // First check if they have the permission node via the permission plugin
                    let has_node = if let Some(checker) = crate::plugin::api::permissions::get_permission_checker() {
                        checker.check_permission(&player.gameprofile.id, permission)
                    } else {
                        false
                    };

                    // Then check if they have the required permission level
                    let has_level = match permission.strip_prefix("minecraft.command.") {
                        Some("op") | Some("stop") => self.has_permission_lvl(PermissionLvl::Three),
                        Some("gamemode") | Some("tp") | Some("give") => self.has_permission_lvl(PermissionLvl::Two),
                        Some("help") | Some("list") | Some("msg") => self.has_permission_lvl(PermissionLvl::Zero),
                        _ => self.has_permission_lvl(PermissionLvl::Two), // Default to level 2 for unknown commands
                    };

                    // Need both the node and the level
                    return has_node && has_level;
                }
                
                // For plugin commands, use permission checker
                if let Some(checker) = crate::plugin::api::permissions::get_permission_checker() {
                    checker.check_permission(&player.gameprofile.id, permission)
                } else {
                    false // No permission checker means all permissions are denied by default
                }
            }
        }
    }

    pub fn position(&self) -> Option<Vector3<f64>> {
        match self {
            CommandSender::Console | CommandSender::Rcon(..) => None,
            CommandSender::Player(p) => Some(p.living_entity.entity.pos.load()),
        }
    }

    pub fn world(&self) -> Option<&World> {
        match self {
            // TODO: maybe return first world when console
            CommandSender::Console | CommandSender::Rcon(..) => None,
            CommandSender::Player(p) => Some(&p.living_entity.entity.world),
        }
    }
}

#[must_use]
pub fn default_dispatcher() -> CommandDispatcher {
    let mut dispatcher = CommandDispatcher::new();

    // Register all core commands (using two-argument version)
    dispatcher.register(pumpkin::init_command_tree(), PermissionLvl::Zero);
    dispatcher.register(bossbar::init_command_tree(), PermissionLvl::Two);
    dispatcher.register(say::init_command_tree(), PermissionLvl::Two);
    dispatcher.register(gamemode::init_command_tree(), PermissionLvl::Two);
    dispatcher.register(stop::init_command_tree(), PermissionLvl::Four);
    dispatcher.register(help::init_command_tree(), PermissionLvl::Zero);
    dispatcher.register(kill::init_command_tree(), PermissionLvl::Two);
    dispatcher.register(kick::init_command_tree(), PermissionLvl::Three);
    dispatcher.register(plugin::init_command_tree(), PermissionLvl::Three);
    dispatcher.register(plugins::init_command_tree(), PermissionLvl::Three);
    dispatcher.register(worldborder::init_command_tree(), PermissionLvl::Two);
    dispatcher.register(teleport::init_command_tree(), PermissionLvl::Two);
    dispatcher.register(time::init_command_tree(), PermissionLvl::Two);
    dispatcher.register(give::init_command_tree(), PermissionLvl::Two);
    dispatcher.register(list::init_command_tree(), PermissionLvl::Zero);      
    dispatcher.register(clear::init_command_tree(), PermissionLvl::Two);      
    dispatcher.register(setblock::init_command_tree(), PermissionLvl::Two);   
    dispatcher.register(seed::init_command_tree(), PermissionLvl::Two);       
    dispatcher.register(transfer::init_command_tree(), PermissionLvl::Zero);  
    dispatcher.register(fill::init_command_tree(), PermissionLvl::Two);       
    dispatcher.register(op::init_command_tree(), PermissionLvl::Three);       
    dispatcher.register(deop::init_command_tree(), PermissionLvl::Three);     
    dispatcher.register(me::init_command_tree(), PermissionLvl::Zero);        
    dispatcher.register(playsound::init_command_tree(), PermissionLvl::Two);  
    dispatcher.register(title::init_command_tree(), PermissionLvl::Two);      
    dispatcher.register(summon::init_command_tree(), PermissionLvl::Two);     
    dispatcher.register(msg::init_command_tree(), PermissionLvl::Zero);       
    dispatcher.register(ban::init_command_tree(), PermissionLvl::Three);      
    dispatcher.register(banip::init_command_tree(), PermissionLvl::Three);    
    dispatcher.register(banlist::init_command_tree(), PermissionLvl::Three);  
    dispatcher.register(pardon::init_command_tree(), PermissionLvl::Three);   
    dispatcher.register(pardonip::init_command_tree(), PermissionLvl::Three);

    dispatcher
}

#[async_trait]
pub trait CommandExecutor: Sync {
    async fn execute<'a>(
        &self,
        sender: &mut CommandSender<'a>,
        server: &Server,
        args: &ConsumedArgs<'a>,
    ) -> Result<(), CommandError>;
}
