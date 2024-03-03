pub mod viewer;

use std::{any::Any, collections::VecDeque, sync::Arc};

use egui::{mutex::Mutex, RichText};
use egui_extras::Column;
use indexmap::IndexSet;

use crate::{RowSlotId, Spreadsheet};
pub use viewer::RowViewer;

use format as f;

/* ------------------------------------------ Indexing ------------------------------------------ */

type ColumnIndex = usize;
type LinearCellIndex = usize;
type AnyRowValue = Box<dyn Any + Send>;

#[derive(Default)]
struct UiState {
    /// Cached sheet id. If sheet id mismatches, the UiState is invalidated.
    sheet_id: u64,

    /// Cached number of columns.
    num_columns: usize,

    /// Visible columns selected by user.
    visible_cols: Vec<ColumnIndex>,

    /// Sort option - column index and direction.
    sort_by: Option<ColumnIndex>,

    /// Row selections.
    selections: IndexSet<LinearCellIndex>,

    /// When activated, previous row value is cached here
    ///
    /// TODO: undo / redo
    active_cell_source_value: Option<Box<dyn Any + Send>>,

    /// Any modification is stored here. We assume this is transient(putting this as field
    /// of UiState), as it's brittle to external modification to the spreadsheet.
    undo_stack: VecDeque<Command>,

    /// Any redo will push back to undo stack. This will be cleared when any modification
    /// is made.
    redo_stack: Vec<Command>,

    /*

        SECTION: Cache - Rendering

    */
    /// Cached rows.
    cc_rows: Vec<(RowSlotId, f32)>,

    /// Spreadsheet is modified during the last validation.
    cc_dirty: bool,
}

#[derive(Clone)]
enum Command {}

impl UiState {
    fn validate_identity(&mut self, id: u64, num_columns: usize) {
        if self.sheet_id == id && self.num_columns == num_columns {
            return;
        }

        // Clear the cache
        *self = Default::default();
        self.sheet_id = id;
        self.cc_dirty = true;
        self.num_columns = num_columns;
        self.visible_cols.extend(0..num_columns);
    }

    fn validate_cc<R: Send, V: RowViewer<R>>(&mut self, sheet: &mut Spreadsheet<R>, vwr: &mut V) {
        if !self.cc_dirty {
            return;
        }

        // We should validate the entire cache.
    }
}

/* ------------------------------------------ Rendering ----------------------------------------- */

impl<R: Send> Spreadsheet<R> {
    /// Show the spreadsheet.
    ///
    /// You should be careful on assigning `ui_id` to this spreadsheet. If it is
    /// duplicated with any other spreadsheet that is actively rendered, it'll constantly
    /// invalidate the index table cache which results in a performance hit.
    pub fn show<V>(&mut self, ui: &mut egui::Ui, ui_id: impl Into<egui::Id>, viewer: &mut V)
    where
        R: Send,
        V: RowViewer<R>,
    {
        let ui_id = ui_id.into();
        let ui_state_ptr = ui
            .memory(|x| x.data.get_temp::<Arc<Mutex<UiState>>>(ui_id))
            .unwrap_or_default();

        let mut ui_state = ui_state_ptr.lock();
        ui_state.validate_identity(self.unique_id, viewer.num_columns());

        ui.push_id(ui_id, |ui| {
            self.show_impl(ui, &mut ui_state, viewer);
        });

        // Reset memory.
        drop(ui_state);
        ui.memory_mut(|x| x.data.insert_temp(ui_id, ui_state_ptr));
    }

    fn show_impl<V>(&mut self, ui: &mut egui::Ui, s: &mut UiState, viewer: &mut V)
    where
        R: Send,
        V: RowViewer<R>,
    {
        let mut added_commands = Vec::<Command>::new();
        let mut undo = false;
        let mut redo = false;

        egui_extras::TableBuilder::new(ui)
            .column(Column::auto().resizable(false))
            .columns(Column::auto().resizable(true), s.visible_cols.len())
            .drag_to_scroll(false) // Drag is used for selection
            .striped(true)
            .sense(egui::Sense::click_and_drag())
            .header(20., |mut h| {
                for &col in &s.visible_cols {
                    h.col(|ui| {
                        ui.label(viewer.column_name(col));
                    });
                }
            })
            .body(|body| {
                // Validate ui state. Defer this as late as possible; since it may not be
                // called if the table area is out of the visible space.
                s.validate_cc(self, viewer);

                let mut row_len_updates = Vec::new();

                body.heterogeneous_rows(s.cc_rows.iter().map(|(_, a)| *a), |mut row| {
                    let row_index = row.index();
                    let (row_slot_id, row_height_prev) = s.cc_rows[row_index];

                    // Render row header button
                    let (mut rect, mut resp) = row.col(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::from(f!("{row_slot_id}")).monospace().strong());
                            ui.add_space(0.);
                            ui.label(RichText::from(f!("({row_index})")).monospace().weak());
                        });
                    });

                    // Refresh height of the row.
                    let mut max_cell_height = rect.height();

                    for &col in &s.visible_cols {
                        let linear_index = row_index * s.visible_cols.len() + col;
                        let selected = s.selections.contains(&linear_index);
                        let is_active = s.active_cell_source_value.is_some()
                            && selected
                            && s.selections.first() == Some(&linear_index);

                        row.set_selected(selected);

                        (rect, resp) = row.col(|ui| {
                            ui.add_enabled_ui(is_active, |ui| {
                                viewer.draw_column_edit(
                                    ui,
                                    &mut self.rows[row_slot_id],
                                    col,
                                    is_active,
                                );
                            });
                        });

                        max_cell_height = rect.height().max(max_cell_height);

                        // TODO: Create actions from response.
                        // - Highlight on click
                        // - Ctrl + Click, Shift + Click, Ctrl + A, Drag.
                        // -
                    }

                    if row_height_prev != max_cell_height {
                        row_len_updates.push((row_index, max_cell_height));
                    }
                });

                // Update height caches

                for (row_index, row_height) in row_len_updates {
                    s.cc_rows[row_index].1 = row_height;
                }
            });
    }
}
