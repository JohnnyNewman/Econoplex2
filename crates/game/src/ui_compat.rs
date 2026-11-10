//! Button state for the panels, on Bevy 0.20's UI widgets.
//!
//! Bevy 0.20 replaced `Button` / `Interaction` with the headless
//! `ui_widgets::Button`, picking's `Hovered`, and the `Pressed` marker. The panels
//! only need to know "none / hovered / pressed" per button, so this keeps that
//! small state ourselves, updated from the new components, and only changes it
//! when it really changes (so `Changed<Interaction>` keeps working).

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::Pressed;

pub use bevy::ui_widgets::Button;

/// What the pointer is doing with a button.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interaction {
    #[default]
    None,
    Hovered,
    Pressed,
}

/// The components a clickable panel element needs.
pub fn button() -> impl Bundle {
    (Button, Hovered::default(), Interaction::default())
}

pub fn update_interaction(mut q: Query<(&mut Interaction, Has<Pressed>, &Hovered), With<Button>>) {
    for (mut i, pressed, hovered) in &mut q {
        let now = if pressed {
            Interaction::Pressed
        } else if hovered.get() {
            Interaction::Hovered
        } else {
            Interaction::None
        };
        // Only write on a change, so change detection reports real changes.
        if *i != now {
            *i = now;
        }
    }
}
