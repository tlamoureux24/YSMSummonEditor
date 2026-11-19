//! Everything a view or dialog can ask the application to do.
//!
//! Views never mutate state themselves: they push actions, and
//! `app::actions` applies them after the frame has been drawn.

use ysm_core::commit::SkillUpdate;
use ysm_core::model::{EntryField, SkillDisplay, SummonListItem};
use ysm_core::random_group::RandomSkillGroup;
use ysm_core::save_plan::SaveAnswer;
use ysm_core::scaling_edit::ScalingEditResult;
use ysm_core::session::TreeRef;
use ysm_data::AppSettings;

use crate::widgets::toast::ToastKind;
use crate::worker::Command;

/// What the skill picker was opened for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerPurpose {
    /// Append a new row with the picked skill.
    AddRow(TreeRef),
    /// Replace the skill of an existing row.
    ChangeSkill(TreeRef, u64),
}

pub enum Action {
    /// Forward a request to the worker thread (used by dialogs that search).
    Command(Command),
    Toast(ToastKind, String),
    CopyText(String),

    // ---- summon list ----
    Search,
    FocusSearch,
    /// The user clicked a summon; guarded by the unsaved-changes prompt.
    SelectSummon(SummonListItem),
    /// Load a summon, dropping whatever is staged.
    SwitchSummon(SummonListItem),
    SaveAndSwitch(SummonListItem),
    /// Reload the current chain from the database.
    Reload,

    // ---- toolbar ----
    Save,
    Undo,
    Redo,
    AskDiscardAll,
    DiscardAll,
    OpenStats,
    OpenSettings,
    /// Drop the current connection and connect again with the saved settings.
    Reconnect,
    SetSaveToSql(bool),

    // ---- tree panel ----
    SetStage(u8),
    SetSlot(u8),
    SelectRow(Option<u64>),
    SelectEffective(Option<(i32, i32)>),
    AddRow(TreeRef),
    OpenSkillPicker(PickerPurpose),
    SkillPicked {
        purpose: PickerPurpose,
        skill: SkillDisplay,
    },
    DuplicateRow(TreeRef, u64),
    AskDeleteRow(TreeRef, u64),
    DeleteRow(TreeRef, u64),
    SetField {
        tree: TreeRef,
        uid: u64,
        field: EntryField,
        value: i32,
    },
    SetEffectiveLevels {
        stage: u8,
        skill_id: i32,
        lv: i32,
        min: i32,
        max: i32,
    },
    AskDeleteEffective {
        stage: u8,
        skill_id: i32,
        lv: i32,
    },
    DeleteEffective {
        stage: u8,
        skill_id: i32,
        lv: i32,
    },

    // ---- skills, groups, stats ----
    EditSkill(i32),
    StageSkillUpdate(SkillUpdate),
    CreateGroup(TreeRef, u64),
    EditGroup(TreeRef, u64),
    AskDetachGroup(TreeRef, u64),
    DetachGroup(TreeRef, u64),
    ApplyRandomGroup {
        attach_to: Option<(TreeRef, u64)>,
        group: RandomSkillGroup,
    },
    ApplyScaling(ScalingEditResult),

    // ---- settings ----
    ApplySettings(Box<AppSettings>),

    // ---- save flow ----
    SaveAnswer(SaveAnswer),
    SaveCancelled,
    ReviewConfirmed,
    CommitConfirmed,

    // ---- closing the window ----
    /// The window was asked to close while changes are staged.
    AskExit,
    CancelExit,
    /// Close the window, discarding staged changes.
    Exit,
}
