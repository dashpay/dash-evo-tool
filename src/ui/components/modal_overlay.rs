/// Returns true if the user left-clicked outside the given window rect this frame.
/// Use after painting a modal overlay and showing the dialog window.
pub fn clicked_outside_window(ctx: &egui::Context, window_rect: egui::Rect) -> bool {
    ctx.input(|i| {
        i.pointer.primary_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|pos| !window_rect.contains(pos))
    })
}
