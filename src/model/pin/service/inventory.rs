//! Inventory listing — build flat and grouped `PinInventoryItem`s from the
//! Kubo pinset plus persistent watched-pin records, and paginated variants.

use super::work::build_work_inventory_item;
use crate::{
    AppState,
    model::{
        config::types::BridgeConfig,
        pin::{
            client::kubo::list_kubo_pinset,
            inventory::{build_single_inventory_item, collect_inventory_descriptors},
            types::{InventoryEntryDescriptor, PinInventoryItem, PinsPageResponse, PinsResponse},
        },
    },
};

pub async fn build_inventory_item_from_descriptor(
    state: &AppState,
    config: &BridgeConfig,
    descriptor: &InventoryEntryDescriptor,
) -> PinInventoryItem {
    match descriptor {
        InventoryEntryDescriptor::Single(source) => {
            build_single_inventory_item(config, source.clone())
        }
        InventoryEntryDescriptor::Work(members) => {
            build_work_inventory_item(state, config, members).await
        }
    }
}

pub async fn list_local_pin_inventory(state: &AppState) -> anyhow::Result<PinsResponse> {
    let pinset = list_kubo_pinset(state).await?;
    let persistent = state.persistent.read().await.clone();
    let config = state.config.read().await.clone();
    let descriptors = collect_inventory_descriptors(&pinset, &persistent);
    let mut items = Vec::with_capacity(descriptors.len());
    for descriptor in &descriptors {
        items.push(build_inventory_item_from_descriptor(state, &config, descriptor).await);
    }

    Ok(PinsResponse {
        total: descriptors.len(),
        pinned_count: descriptors.iter().filter(|descriptor| descriptor.pinned()).count(),
        managed_count: descriptors.len(),
        last_repair_cycle_at: persistent.last_repair_cycle_at,
        items,
    })
}

pub async fn list_local_pin_inventory_page(
    state: &AppState,
    cursor: usize,
    limit: usize,
) -> anyhow::Result<PinsPageResponse> {
    let pinset = list_kubo_pinset(state).await?;
    let persistent = state.persistent.read().await.clone();
    let config = state.config.read().await.clone();
    let descriptors = collect_inventory_descriptors(&pinset, &persistent);

    let total = descriptors.len();
    let start = cursor.min(total);
    let end = start.saturating_add(limit).min(total);
    let pinned_count = descriptors.iter().filter(|descriptor| descriptor.pinned()).count();

    let mut items = Vec::with_capacity(end.saturating_sub(start));
    for descriptor in &descriptors[start..end] {
        items.push(build_inventory_item_from_descriptor(state, &config, descriptor).await);
    }

    Ok(PinsPageResponse {
        total,
        pinned_count,
        managed_count: total,
        next_cursor: (end < total).then(|| end.to_string()),
        items,
    })
}
