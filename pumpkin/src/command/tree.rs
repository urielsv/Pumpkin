use super::{args::ArgumentConsumer, CommandExecutor};
use crate::command::CommandSender;
use pumpkin_util::permission::PermissionChecker;
use std::{collections::VecDeque, fmt::Debug, sync::{Arc, OnceLock}};

/// see [`crate::commands::tree_builder::argument`]
pub type RawArgs<'a> = Vec<&'a str>;

#[derive(Debug, Clone)]
pub struct Node {
    pub(crate) children: Vec<usize>,
    pub(crate) node_type: NodeType,
    pub(crate) requires_permission: bool,
}

#[derive(Clone)]
pub enum NodeType {
    ExecuteLeaf {
        executor: Arc<dyn CommandExecutor + Send>,
    },
    Literal {
        string: String,
    },
    Argument {
        name: String,
        consumer: Arc<dyn ArgumentConsumer + Send>,
    },
    Require {
        predicate: Arc<dyn Fn(&CommandSender) -> bool + Send + Sync>,
    },
}

impl Debug for NodeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExecuteLeaf { .. } => f
                .debug_struct("ExecuteLeaf")
                .field("executor", &"..")
                .finish(),
            Self::Literal { string } => f.debug_struct("Literal").field("string", string).finish(),
            Self::Argument { name, .. } => f
                .debug_struct("Argument")
                .field("name", name)
                .field("consumer", &"..")
                .finish(),
            Self::Require { .. } => f.debug_struct("Require").field("predicate", &"..").finish(),
        }
    }
}

pub enum Command {
    Tree(CommandTree),
    Alias(String),
}

#[derive(Debug, Clone)]
pub struct CommandTree {
    pub(crate) nodes: Vec<Node>,
    pub(crate) children: Vec<usize>,
    pub(crate) names: Vec<String>,
    pub(crate) description: String,
}

impl CommandTree {
    /// iterate over all possible paths that end in a [`NodeType::ExecuteLeaf`]
    pub(crate) fn iter_paths(&self) -> impl Iterator<Item = Vec<usize>> + use<'_> {
        let mut todo = VecDeque::<(usize, usize)>::new();

        // add root's children
        todo.extend(self.children.iter().map(|&i| (0, i)));

        TraverseAllPathsIter {
            tree: self,
            path: Vec::<usize>::new(),
            todo,
        }
    }

    /// get the permission name for a node based on its name in lowercase
    fn get_node_permission_name(&self, node_index: usize) -> Option<String> {
        let node = &self.nodes[node_index];
        match &node.node_type {
            NodeType::Literal { string } => Some(string.to_lowercase()),
            NodeType::Argument { name, .. } => Some(name.to_lowercase()),
            _ => None
        }
    }

    /// get the permission string for a node including plugin name (e.g. pluginname.command.subcommand)
    pub fn get_permission(&self, node_index: usize, plugin_name: &str) -> Option<String> {
        let mut current_index = node_index;
        let mut permission_parts = vec![plugin_name.to_string()];
        
        // collect permissions from the current node up to the root
        while let Some(_) = self.nodes.get(current_index) {
            if let Some(perm) = self.get_node_permission_name(current_index) {
                permission_parts.push(perm);
            }
            
            current_index = self.find_parent(current_index)?;
        }
        
        // reverse to get root-to-leaf order (except plugin_name which stays first)
        permission_parts[1..].reverse();
        
        if permission_parts.len() <= 1 {
            None
        } else {
            Some(permission_parts.join("."))
        }
    }

    /// find the parent node index for a given node
    fn find_parent(&self, node_index: usize) -> Option<usize> {
        for (i, node) in self.nodes.iter().enumerate() {
            if node.children.contains(&node_index) {
                return Some(i);
            }
        }
        None
    }

    /// check if a command requires permission and if the sender has it
    pub fn check_permission(&self, path: &[usize], sender: &CommandSender, plugin_name: &str) -> bool {
        // get the last node (the execute leaf)
        let last_node = &self.nodes[*path.last().unwrap()];
        
        if !last_node.requires_permission {
            return true;
        }

        // get the permission string
        if let Some(permission) = self.get_permission(*path.last().unwrap(), plugin_name) {
            match sender {
                CommandSender::Player(player) => {
                    // Use the registered permission checker if one exists
                    if let Some(checker) = PERMISSION_CHECKER.get() {
                        checker.check_permission(&player.gameprofile.id, &permission)
                    } else {
                        true // Default to allowing if no permission system is registered
                    }
                }
                CommandSender::Console => true, 
                _ => false,
            }
        } else {
            true // no permission path found
        }
    }
}

struct TraverseAllPathsIter<'a> {
    tree: &'a CommandTree,
    path: Vec<usize>,
    /// (depth, i)
    todo: VecDeque<(usize, usize)>,
}

impl Iterator for TraverseAllPathsIter<'_> {
    type Item = Vec<usize>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (depth, i) = self.todo.pop_front()?;
            let node = &self.tree.nodes[i];

            // add new children to front
            self.todo.reserve(node.children.len());
            node.children
                .iter()
                .rev()
                .for_each(|&c| self.todo.push_front((depth + 1, c)));

            // update path
            while self.path.len() > depth {
                self.path.pop();
            }
            self.path.push(i);

            if let NodeType::ExecuteLeaf { .. } = node.node_type {
                return Some(self.path.clone());
            }
        }
    }
}

static PERMISSION_CHECKER: OnceLock<Arc<dyn PermissionChecker>> = OnceLock::new();

pub fn register_permission_checker(checker: Arc<dyn PermissionChecker>) {
    let _ = PERMISSION_CHECKER.set(checker);
}
