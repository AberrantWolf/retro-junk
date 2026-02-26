//! Spinner pool for concurrent progress display.
//!
//! Manages a fixed number of progress bar "slots" that can be claimed and
//! released by concurrent tasks identified by a `usize` key.

use std::collections::HashMap;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

/// A pool of reusable spinner slots for displaying concurrent task progress.
pub struct SpinnerPool {
    #[allow(dead_code)]
    mp: MultiProgress,
    spinners: Vec<ProgressBar>,
    slot_assignments: HashMap<usize, usize>,
    free_slots: Vec<usize>,
}

impl SpinnerPool {
    /// Create a new spinner pool with `n` slots.
    ///
    /// When `quiet` is true, all spinners are hidden.
    /// When `auto_tick` is true, spinners tick immediately on creation
    /// (used by scrape). When false, spinners stay invisible until claimed
    /// (used by enrich to avoid ghost lines between platforms).
    pub fn new(n: usize, quiet: bool, auto_tick: bool) -> Self {
        let mp = if quiet {
            MultiProgress::with_draw_target(indicatif::ProgressDrawTarget::hidden())
        } else {
            MultiProgress::new()
        };

        let spinner_style = ProgressStyle::with_template("  {spinner:.cyan} {msg}")
            .expect("static pattern")
            .tick_chars("/-\\|");

        let spinners: Vec<ProgressBar> = (0..n)
            .map(|_| {
                let pb = mp.add(ProgressBar::new_spinner());
                pb.set_style(spinner_style.clone());
                if auto_tick {
                    pb.enable_steady_tick(std::time::Duration::from_millis(100));
                }
                pb
            })
            .collect();

        let free_slots = (0..n).rev().collect();

        Self {
            mp,
            spinners,
            slot_assignments: HashMap::new(),
            free_slots,
        }
    }

    /// Claim a spinner slot for the given key and set its message.
    pub fn claim(&mut self, key: usize, msg: String) {
        if let Some(slot) = self.free_slots.pop() {
            self.spinners[slot].reset();
            self.spinners[slot].enable_steady_tick(std::time::Duration::from_millis(100));
            self.spinners[slot].set_message(msg);
            self.slot_assignments.insert(key, slot);
        }
    }

    /// Update the message for a claimed slot. No-op if the key has no slot.
    pub fn update(&self, key: usize, msg: String) {
        if let Some(&slot) = self.slot_assignments.get(&key) {
            self.spinners[slot].set_message(msg);
        }
    }

    /// Release a spinner slot: stop ticking, clear the line, return to pool.
    pub fn release(&mut self, key: usize) {
        if let Some(slot) = self.slot_assignments.remove(&key) {
            self.spinners[slot].disable_steady_tick();
            self.spinners[slot].set_message("");
            self.spinners[slot].finish_and_clear();
            self.free_slots.push(slot);
        }
    }

    /// Clear all spinners and reset slot tracking.
    pub fn clear_all(&mut self) {
        for spinner in &self.spinners {
            spinner.disable_steady_tick();
            spinner.set_message("");
            spinner.finish_and_clear();
        }
        self.slot_assignments.clear();
        self.free_slots = (0..self.spinners.len()).rev().collect();
    }
}
