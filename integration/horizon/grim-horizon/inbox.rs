fn inbox(hub: &mut Hub, i: usize, ctx: &mut TickCtx<'_>, events: &mut Vec<HostEvent>) {
    grim_horizon::process_inbox(hub, i, ctx, events);
}
