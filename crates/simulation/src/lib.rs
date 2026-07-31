pub const TARGET_TICKS_PER_SECOND: u32 = 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TickPhase {
    Input,
    Events,
    Entities,
    Collision,
    Effects,
    Output,
    PersistenceCommit,
}
