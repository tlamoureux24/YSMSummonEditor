//! Semantic names for the Phosphor icons used across the UI.
//!
//! Views refer to these instead of raw glyph names, so swapping an icon is a
//! one-line change here.

use egui_phosphor::regular as ph;

pub const SAVE: &str = ph::FLOPPY_DISK;
pub const UNDO: &str = ph::ARROW_COUNTER_CLOCKWISE;
pub const REDO: &str = ph::ARROW_CLOCKWISE;
pub const DISCARD: &str = ph::ARROW_U_UP_LEFT;
pub const STATS: &str = ph::CHART_LINE_UP;
pub const SETTINGS: &str = ph::GEAR_SIX;
pub const DATABASE: &str = ph::DATABASE;
pub const SQL_FILE: &str = ph::FILE_SQL;
pub const REFRESH: &str = ph::ARROWS_CLOCKWISE;

pub const SEARCH: &str = ph::MAGNIFYING_GLASS;
pub const FILTER: &str = ph::FUNNEL;
pub const CLOSE: &str = ph::X;
pub const CHECK: &str = ph::CHECK;

pub const ADD: &str = ph::PLUS;
pub const ADD_SKILL: &str = ph::LIST_PLUS;
pub const DUPLICATE: &str = ph::COPY;
pub const DELETE: &str = ph::TRASH;
pub const EDIT: &str = ph::PENCIL_SIMPLE;
pub const SWAP: &str = ph::SWAP;
pub const RANDOM_GROUP: &str = ph::DICE_FIVE;
pub const DETACH: &str = ph::LINK_BREAK;

pub const TREE: &str = ph::TREE_STRUCTURE;
pub const EFFECTIVE: &str = ph::LIST_CHECKS;
pub const ARROW_RIGHT: &str = ph::ARROW_RIGHT;
pub const CARD: &str = ph::IDENTIFICATION_CARD;
pub const MOUNT: &str = ph::HORSE;

pub const INFO: &str = ph::INFO;
pub const QUESTION: &str = ph::QUESTION;
pub const WARNING: &str = ph::WARNING;
pub const ERROR: &str = ph::X_CIRCLE;
pub const SUCCESS: &str = ph::CHECK_CIRCLE;

pub const FOLDER: &str = ph::FOLDER_OPEN;
pub const SHOW: &str = ph::EYE;
pub const HIDE: &str = ph::EYE_SLASH;
pub const CONNECTION: &str = ph::PLUGS_CONNECTED;
pub const THEME_DARK: &str = ph::MOON;
pub const THEME_LIGHT: &str = ph::SUN;
pub const THEME_SYSTEM: &str = ph::DESKTOP;
pub const NO_CONNECTION: &str = ph::PLUGS;
pub const IMAGES: &str = ph::IMAGES;
pub const CLIPBOARD: &str = ph::CLIPBOARD_TEXT;
pub const CODE: &str = ph::CODE;
pub const CHANGES: &str = ph::STACK;

pub const INSERTED: &str = ph::PLUS_CIRCLE;
pub const DELETED: &str = ph::MINUS_CIRCLE;
