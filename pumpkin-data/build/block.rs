use std::collections::HashMap;

use heck::ToPascalCase;
use proc_macro2::{Punct, TokenStream};
use quote::{quote, ToTokens, TokenStreamExt};
use serde::Deserialize;
use syn::Field;

use crate::ident;

#[derive(Deserialize)]
struct JSONStruct {
    pub blocks: Vec<JSONBlock>,
}

#[derive(Deserialize, Clone, Debug)]
struct JSONBlock {
    pub id: u16,
    pub item_id: u16,
    pub hardness: f32,
    pub wall_variant_id: Option<u16>,
    pub translation_key: String,
    pub name: String,
    pub properties: Vec<Property>,
    pub default_state_id: u16,
    pub states: Vec<BlockState>,
}

#[derive(Deserialize, Clone, Debug)]
struct Block {
    pub id: u16,
    pub item_id: u16,
    pub hardness: f32,
    pub wall_variant_id: Option<u16>,
    pub translation_key: String,
    pub name: String,
    // pub properties: Vec<Property>,
    pub default_state: BlockState,
    //  pub states: Vec<BlockState>,
}

#[expect(dead_code)]
#[derive(Deserialize, Clone, Debug)]
pub struct Property {
    name: String,
    values: Vec<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct BlockState {
    pub id: u16,
    pub air: bool,
    pub luminance: u8,
    pub burnable: bool,
    pub opacity: Option<u32>,
    pub replaceable: bool,
    pub collision_shapes: Vec<u16>,
    pub block_entity_type: Option<u32>,
}

impl ToTokens for BlockState {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let id = self.id;
        let air = self.air;
        let luminance = self.luminance;
        let burnable = self.burnable;
        let opacity = self.opacity;
        let replaceable = self.replaceable;
        let block_entity_type = self.block_entity_type;
        
        tokens.append_all(quote! {
            BlockState {
            id: #id,
            air: #air,
            luminance: #luminance,
            burnable: #burnable,
            opacity: #opacity,
            replaceable: #replaceable,
            block_entity_type: #block_entity_type,
        }});
    }
}

pub(crate) fn build() -> TokenStream {
    println!("cargo:rerun-if-changed=assets/blocks.json");

    let json: JSONStruct = serde_json::from_str(include_str!("../../assets/blocks.json"))
        .expect("Failed to parse blocks.json");
    let mut variants = TokenStream::new();

    for block in &json.blocks {
        let id = block.id as u16;
        let item_id = block.item_id;
        let hardness = block.hardness;
        let wall_variant_id = block.wall_variant_id;
        let translation_key = block.translation_key.clone();
        let enum_name = ident(block.name.to_pascal_case());
        
        let name = &block.name;
        let properties = &block.properties;
        let states = block.states.clone();
        let state = block
            .states
            .iter()
            .find(|s| s.id == block.default_state_id)
            .expect("Failed to find default state");
        variants.extend([quote! {
            pub const #enum_name: Block = Block {
                id: #id,
                item_id: #item_id,
                hardness: #hardness,
                translation_key: #translation_key,
                name: #name,
                default_state: #state,
            };
        }]);
    }

    let type_from_raw_id_arms = json
        .blocks
        .iter()
        .map(|b| {
            let id = &b.id;
            let name = ident(b.name.to_pascal_case());

            quote! {
                #id => Some(Self::#name),
            }
        })
        .collect::<TokenStream>();

    quote! {
        #[derive(Clone, Debug)]
        struct Block {
            pub id: u16,
            pub item_id: u16,
            pub hardness: f32,
          //  pub wall_variant_id: Option<u16>,
            pub translation_key: String,
            pub name: String,
           // pub properties: Vec<Property>,
            pub default_state: BlockState,
          //  pub states: Vec<BlockState>,
        }

        #[expect(dead_code)]
        #[derive(Clone, Debug)]
        pub struct Property {
            name: String,
            values: Vec<String>,
        }

        #[derive(Clone, Debug)]
        pub struct BlockState {
            pub id: u16,
            pub air: bool,
            pub luminance: u8,
            pub burnable: bool,
            pub opacity: Option<u32>,
            pub replaceable: bool,
            pub collision_shapes: Vec<u16>,
            pub block_entity_type: Option<u32>,
        }

        #variants

        impl Block {
            pub const fn from_raw(id: u16) -> Option<Self> {
                match id {
                    #type_from_raw_id_arms
                    _ => None
                }
            }
        }
    }
}
