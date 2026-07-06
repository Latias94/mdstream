use crate::boundary::{BoundaryPlugin, BoundaryUpdate};

#[derive(Default)]
pub(crate) struct BoundaryRegistry {
    plugins: Vec<Box<dyn BoundaryPlugin>>,
    active: Option<usize>,
}

impl std::fmt::Debug for BoundaryRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundaryRegistry")
            .field("len", &self.plugins.len())
            .field("active", &self.active)
            .finish()
    }
}

impl BoundaryRegistry {
    pub(crate) fn push<T>(&mut self, plugin: T)
    where
        T: BoundaryPlugin + 'static,
    {
        self.plugins.push(Box::new(plugin));
    }

    pub(crate) fn set_active(&mut self, index: usize) {
        self.active = Some(index);
    }

    pub(crate) fn clear_active(&mut self) {
        self.active = None;
    }

    pub(crate) fn start_index(&self, line: &str) -> Option<usize> {
        self.plugins
            .iter()
            .position(|plugin| plugin.matches_start(line))
    }

    pub(crate) fn matches_start(&self, line: &str) -> bool {
        self.start_index(line).is_some()
    }

    pub(crate) fn contains(&self, index: usize) -> bool {
        index < self.plugins.len()
    }

    pub(crate) fn start(&mut self, index: usize, line: &str) {
        self.plugins[index].start(line);
    }

    pub(crate) fn update(&mut self, index: usize, line: &str) -> BoundaryUpdate {
        self.plugins[index].update(line)
    }

    pub(crate) fn reset_all(&mut self) {
        for plugin in &mut self.plugins {
            plugin.reset();
        }
        self.active = None;
    }
}
