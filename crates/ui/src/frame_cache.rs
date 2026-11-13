//! Per-pane caches (textures, decoded pictures, arranged thumbnail lists) are kept in egui's
//! temporary data, which egui never clears by itself. Each cache says it was used this frame;
//! the app calls [`sweep`] once per frame, after every pane is drawn, and whatever was not
//! drawn (a closed workspace or pane, a hidden tab, a grid cell that went away) is dropped,
//! releasing its GPU texture and pixels. A hidden tab re-uploads its textures when shown
//! again; the decoded data itself stays in the engine.

use std::collections::HashMap;

use egui::util::IdTypeMap;
use egui::{Context, Id};

type Remover = fn(&mut IdTypeMap, Id);

#[derive(Clone, Default)]
struct Registry {
    /// Cache entry → how to remove it, and the sweep it was last used in.
    entries: HashMap<Id, (Remover, u64)>,
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
    ctx.data_mut(|data| {
        let registry = data.get_temp_mut_or_default::<Registry>(registry_id());
        let sweep = registry.sweep;
        registry.entries.insert(id, (remove::<T> as Remover, sweep));
    });
}

/// Drop every registered cache that was not used since the previous sweep. Call once per
/// frame, after all panes are drawn.
pub fn sweep(ctx: &Context) {
    ctx.data_mut(|data| {
        let mut registry = data.get_temp::<Registry>(registry_id()).unwrap_or_default();
        let current = registry.sweep;
        let stale: Vec<(Id, Remover)> = registry
            .entries
            .iter()
            .filter(|(_, (_, used))| *used != current)
            .map(|(id, (remover, _))| (*id, *remover))
            .collect();
        for (id, remover) in stale {
            remover(data, id);
            registry.entries.remove(&id);
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
}
