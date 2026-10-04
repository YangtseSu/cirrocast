// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Ordered parallel mapping: the mechanics behind a multi-location run.
//!
//! A multi-location fetch must overlap the slow requests without ever reordering the output. The
//! obvious designs both get that wrong: a completion channel collects in *arrival* order, and
//! joining handles in spawn order ties the result to the spawn schedule. Here the results are
//! written into slot `i` for item `i` under a mutex, so the vector's order is the input's order by
//! construction and there is no completion-order channel to get wrong.
//!
//! The helper is deliberately small: [`std::thread::scope`] (no `'static` bounds, no runtime), one
//! atomic counter for work stealing, and no per-task allocation beyond the result. Panics inside
//! the closure are the caller's problem and are not caught — a slot left empty by a panicking
//! worker surfaces as an `Error::Other` only if the scope itself returns, which it does not when a
//! spawned thread panics (the scope resumes the panic).

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::{Error, Result};

/// Maps `f` over `items` with at most `workers` threads, returning one result per item in the
/// input's order.
///
/// `workers` is clamped to `1..=items.len()`; a zero worker count means "run serially", which is
/// also what a single item does without spawning anything. The closure receives the item's index
/// and a reference to the item, so per-item context can be looked up without cloning.
pub fn par_map_ordered<T, U>(
    items: &[T],
    workers: usize,
    f: impl Fn(usize, &T) -> Result<U> + Sync,
) -> Vec<Result<U>>
where
    T: Sync,
    U: Send,
{
    if items.is_empty() {
        return Vec::new();
    }
    let workers = workers.clamp(1, items.len());
    if workers == 1 {
        return items
            .iter()
            .enumerate()
            .map(|(index, item)| f(index, item))
            .collect();
    }

    let next = AtomicUsize::new(0);
    let mut slots: Vec<Option<Result<U>>> = (0..items.len()).map(|_| None).collect();
    let slots = Mutex::new(&mut slots[..]);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else {
                        return;
                    };
                    let result = f(index, item);
                    let mut slots = slots
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let Some(slot) = slots.get_mut(index) {
                        *slot = Some(result);
                    }
                }
            });
        }
    });

    // Every slot was written by exactly one worker; a missing one could only mean a panic, and a
    // panic in a scoped thread is resumed by `scope` before this point. The fallback keeps the
    // function panic-free by construction rather than by argument.
    slots
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter_mut()
        .map(|slot| {
            slot.take().unwrap_or_else(|| {
                Err(Error::Other(
                    "internal: a parallel worker left its result slot empty".to_owned(),
                ))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::par_map_ordered;
    use crate::error::Error;

    #[test]
    fn results_follow_the_input_order_whatever_the_delays_are() {
        // Descending delays: the last item finishes first, the first item last. A completion-order
        // collector would reverse this; the slot writes must not.
        let items: Vec<usize> = (0..8).collect();
        let results = par_map_ordered(&items, 4, |index, item| {
            let delay = u64::try_from(8 - index).unwrap_or(1);
            std::thread::sleep(std::time::Duration::from_millis(delay * 10));
            Ok((index, *item))
        });
        let mapped: Vec<(usize, usize)> = results
            .into_iter()
            .map(|result| result.expect("no failure"))
            .collect();
        assert_eq!(
            mapped,
            items.iter().map(|item| (*item, *item)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn failures_keep_their_slots() {
        let items = ["ok", "bad", "ok", "bad"];
        let results = par_map_ordered(&items, 3, |_, item| {
            if *item == "bad" {
                Err(Error::LocationNotFound(format!("{item} location")))
            } else {
                Ok(item.len())
            }
        });
        assert_eq!(results.len(), items.len());
        assert_eq!(results[0].as_ref().ok(), Some(&2));
        assert!(matches!(results[1], Err(Error::LocationNotFound(_))));
        assert_eq!(results[2].as_ref().ok(), Some(&2));
        assert!(matches!(results[3], Err(Error::LocationNotFound(_))));
    }

    #[test]
    fn zero_workers_or_one_item_run_serially() {
        let items = [1, 2, 3];
        let results = par_map_ordered(&items, 0, |index, item| Ok((index, *item)));
        assert_eq!(
            results
                .into_iter()
                .map(|result| result.expect("no failure"))
                .collect::<Vec<_>>(),
            vec![(0, 1), (1, 2), (2, 3)]
        );

        let empty: [u8; 0] = [];
        assert!(par_map_ordered(&empty, 4, |_, item| Ok(*item)).is_empty());

        let single = [7_u8];
        assert_eq!(
            par_map_ordered(&single, 8, |_, item| Ok(u16::from(*item)))
                .into_iter()
                .map(|result| result.expect("no failure"))
                .collect::<Vec<_>>(),
            vec![7_u16]
        );
    }
}
