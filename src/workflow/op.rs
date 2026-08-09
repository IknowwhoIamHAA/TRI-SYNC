impl Op {
    pub fn to_event(&self, tenant: String, state: &BinaryStateMap) -> Result<Event> {
        match self {
            Op::Set { key, value } => Event::new_set(tenant, key.clone(), value.clone()),
            Op::Delete { key } => Event::new_delete(tenant, key.clone()),
            Op::Add { key, value } => {
                let current = state.read_f64(key)?;
                let delta: f64 = value.parse()?;
                let new = current + delta;
                Event::new_set_f64(tenant, key.clone(), new)
            }
            Op::Multiply { key, value } => {
                let current = state.read_f64(key)?;
                let factor: f64 = value.parse()?;
                let new = current * factor;
                Event::new_set_f64(tenant, key.clone(), new)
            }
        }
    }
}
