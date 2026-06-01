use bevy::ecs::entity::EntityHashSet;
use bevy::prelude::*;

/// A double [`EntityHashSet`] for storing both used and unused entities.
#[derive(Default, Debug, Clone, Resource)]
pub struct EntityPool {
    used: EntityHashSet,
    unused: EntityHashSet,
}

impl EntityPool {
    pub fn used(&self) -> &EntityHashSet {
        &self.used
    }

    pub fn unused(&self) -> &EntityHashSet {
        &self.unused
    }

    pub fn contains(&self, entity: &Entity) -> bool {
        self.used.contains(entity) || self.unused.contains(entity)
    }

    pub fn insert_new_used(&mut self, entity: Entity) -> bool {
        self.used.insert(entity)
    }

    pub fn insert_new_unused(&mut self, entity: Entity) -> bool {
        self.unused.insert(entity)
    }

    /// Get an unused entity from the entity pool and move it from unused to used.
    pub fn get_unused(&mut self) -> Option<Entity> {
        let entity = self.unused.iter().next().copied()?;
        self.unused.remove(&entity);
        self.used.insert(entity);
        Some(entity)
    }

    /// Get an unused entity or return None. Caller should spawn and call insert_new_used if None.
    pub fn get_unused_or_spawn(&mut self, mut spawn: impl FnMut() -> Entity) -> Entity {
        match self.get_unused() {
            Some(entity) => entity,
            None => {
                let entity = spawn();
                self.insert_new_used(entity);
                entity
            }
        }
    }

    /// Set entity as unused.
    pub fn set_unused(&mut self, entity: Entity) {
        if !self.used.remove(&entity) {
            // warn!("EntityPool: {entity} was not in the used set!"); // noisy
        }
        self.unused.insert(entity);
    }

    /// Drain all used entities and insert them into the unused set.
    pub fn set_all_unused(&mut self) {
        for entity in self.used.drain() {
            self.unused.insert(entity);
        }
    }

    /// Drop all pooled entity ids (e.g. after match teardown when entities were despawned).
    pub fn clear(&mut self) {
        self.used.clear();
        self.unused.clear();
    }
}
