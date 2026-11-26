use cpal::SupportedStreamConfig;
pub(super) fn pick_supported_config<I, R>(
    configs: I,
    target_rate: u32,
    target_channels: u16,
) -> Option<SupportedStreamConfig>
where
    I: IntoIterator<Item = R>,
    R: Into<cpal::SupportedStreamConfigRange>,
{
    let candidates: Vec<cpal::SupportedStreamConfigRange> = configs
        .into_iter()
        .map(Into::into)
        .filter(|cfg| cfg.channels() == target_channels)
        .collect();

    if let Some(exact) = candidates
        .iter()
        .find(|cfg| target_rate >= cfg.min_sample_rate() && target_rate <= cfg.max_sample_rate())
    {
        return Some((*exact).with_sample_rate(target_rate));
    }

    candidates
        .into_iter()
        .map(|cfg| {
            let min = cfg.min_sample_rate();
            let max = cfg.max_sample_rate();
            let clamped = target_rate.clamp(min, max);
            cfg.with_sample_rate(clamped)
        })
        .min_by_key(|cfg| (i64::from(cfg.sample_rate()) - i64::from(target_rate)).abs())
}
