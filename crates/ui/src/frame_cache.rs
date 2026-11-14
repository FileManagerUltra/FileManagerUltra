//! Per-pane caches (textures, decoded pictures, arranged thumbnail lists) are kept in egui's
//! temporary data, which egui never clears by itself. Each cache says it was used this frame;
//! the app calls [`sweep`] once per frame, after every pane is drawn, and whatever was not
//! drawn (a closed workspace or pane, a hidden tab, a grid cell that went away) is dropped,
//! releasing its GPU texture and pixels. A hidden tab re-uploads its textures when shown
//! again; the decoded data itself stays in the engine.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use egui::util::IdTypeMap;
use egui::{Context, Id};
use fm_engine::diag;

type Remover = fn(&mut IdTypeMap, Id);

#[derive(Clone, Copy)]
struct Entry {
    remove: Remover,
    /// The sweep it was last used in.
    used: u64,
    /// When it was last used, and how long it outlives sweeps that did not use it.
    kept: Option<(Instant, Duration)>,
    /// Named entries are reported in the diagnostic log when released.
    label: Option<&'static str>,
}

#[derive(Clone, Default)]
struct Registry {
    entries: HashMap<Id, Entry>,
    sweep: u64,
}

fn registry_id() -> Id {
    Id::NULL.with("fm-frame-cache-registry")
}

fn remove<T: 'static + Clone + Send + Sync>(data: &mut IdTypeMap, id: Id) {
    data.remove::<T>(id);
}

/// Record that the temporary value of type `T` under `id` is in use this frame.
pub(crate) fn touch<T: 'static + Clone + Send + Sync>(ctx: &Context, id: Id) {
    register::<T>(ctx, id, None, None);
}

/// Like [`touch`], but the entry survives `keep` after its last use (a video picture across
/// a seek or decoder restart, when no frame is drawn for a moment). `label` names it in the
/// diagnostic log when it is released.
pub(crate) fn touch_kept<T: 'static + Clone + Send + Sync>(
    ctx: &Context,
    id: Id,
    keep: Duration,
    label: &'static str,
) {
    register::<T>(ctx, id, Some(keep), Some(label));
}

fn register<T: 'static + Clone + Send + Sync>(
    ctx: &Context,
    id: Id,
    keep: Option<Duration>,
    label: Option<&'static str>,
) {
    ctx.data_mut(|data| {
        let registry = data.get_temp_mut_or_default::<Registry>(registry_id());
        let entry = Entry {
            remove: remove::<T>,
            used: registry.sweep,
            kept: keep.map(|keep| (Instant::now(), keep)),
            label,
        };
        registry.entries.insert(id, entry);
    });
}

/// Drop every registered cache that was not used since the previous sweep. Call once per
/// frame, after all panes are drawn.
pub fn sweep(ctx: &Context) {
    ctx.data_mut(|data| {
        let mut registry = data.get_temp::<Registry>(registry_id()).unwrap_or_default();
        let current = registry.sweep;
        let now = Instant::now();
        let stale: Vec<(Id, Entry)> = registry
            .entries
            .iter()
            .filter(|(_, e)| {
                e.used != current
                    && e.kept
                        .is_none_or(|(at, keep)| now.saturating_duration_since(at) >= keep)
            })
            .map(|(id, e)| (*id, *e))
            .collect();
        for (id, entry) in stale {
            (entry.remove)(data, id);
            registry.entries.remove(&id);
            diag::count(diag::Counter::CacheEvicted);
            if let Some(label) = entry.label {
                diag::count(diag::Counter::VideoTexEvicted);
                diag::event("texture", || format!("{label} released (not drawn)"));
            }
        }
        registry.sweep = current.wrapping_add(1);
        data.insert_temp(registry_id(), registry);
    });
}

/// How many caches are registered (for tests).
pub fn live(ctx: &Context) -> usize {
    ctx.data(|data| {
        data.get_temp::<Registry>(registry_id())
            .map_or(0, |r| r.entries.len())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_not_used_since_the_last_sweep_are_dropped() {
        let ctx = Context::default();
        let (a, b) = (Id::NULL.with("a"), Id::NULL.with("b"));
        ctx.data_mut(|d| {
            d.insert_temp(a, 1u32);
            d.insert_temp(b, 2u32);
        });
        touch::<u32>(&ctx, a);
        touch::<u32>(&ctx, b);
        sweep(&ctx);
        assert_eq!(live(&ctx), 2, "used this frame: kept");
        // Next frame only `a` is drawn.
        touch::<u32>(&ctx, a);
        sweep(&ctx);
        assert_eq!(ctx.data(|d| d.get_temp::<u32>(a)), Some(1));
        assert_eq!(
            ctx.data(|d| d.get_temp::<u32>(b)),
            None,
            "not drawn: dropped"
        );
        assert_eq!(live(&ctx), 1);
        sweep(&ctx);
        assert_eq!(ctx.data(|d| d.get_temp::<u32>(a)), None);
        assert_eq!(live(&ctx), 0);
    }

    #[test]
    fn kept_entries_survive_missed_frames_until_their_time_is_up() {
        let ctx = Context::default();
        let (kept, zero) = (Id::NULL.with("kept"), Id::NULL.with("zero"));
        ctx.data_mut(|d| {
            d.insert_temp(kept, 1u32);
            d.insert_temp(zero, 2u32);
        });
        touch_kept::<u32>(&ctx, kept, Duration::from_secs(60), "test");
        touch_kept::<u32>(&ctx, zero, Duration::ZERO, "test");
        sweep(&ctx);
        // Neither is drawn for a few frames: only the one without a grace period goes.
        for _ in 0..5 {
            sweep(&ctx);
        }
        assert_eq!(ctx.data(|d| d.get_temp::<u32>(kept)), Some(1));
        assert_eq!(ctx.data(|d| d.get_temp::<u32>(zero)), None);
        assert_eq!(live(&ctx), 1);
    }
}
