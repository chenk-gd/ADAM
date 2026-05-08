use adam_domain::asset::state::AssetState;

#[test]
fn asset_state_can_transition_from_clean_to_dirty() {
    let state = AssetState::Clean;
    assert!(state.can_transition_to(AssetState::Dirty));
}

#[test]
fn asset_state_can_transition_from_dirty_to_clean() {
    let state = AssetState::Dirty;
    assert!(state.can_transition_to(AssetState::Clean));
}

#[test]
fn asset_state_can_transition_from_dirty_to_archived() {
    let state = AssetState::Dirty;
    assert!(state.can_transition_to(AssetState::Archived));
}

#[test]
fn asset_state_can_transition_from_clean_to_archived() {
    let state = AssetState::Clean;
    assert!(state.can_transition_to(AssetState::Archived));
}

#[test]
fn asset_state_cannot_transition_from_archived_to_any() {
    let state = AssetState::Archived;
    assert!(!state.can_transition_to(AssetState::Clean));
    assert!(!state.can_transition_to(AssetState::Dirty));
    assert!(!state.can_transition_to(AssetState::Archived));
}

#[test]
fn asset_state_is_dirty_returns_true_for_dirty() {
    assert!(AssetState::Dirty.is_dirty());
    assert!(!AssetState::Clean.is_dirty());
    assert!(!AssetState::Archived.is_dirty());
}

#[test]
fn asset_state_is_archived_returns_true_for_archived() {
    assert!(AssetState::Archived.is_archived());
    assert!(!AssetState::Clean.is_archived());
    assert!(!AssetState::Dirty.is_archived());
}
