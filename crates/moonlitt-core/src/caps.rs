bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct BackendCaps: u32 {
        const SOURCE = 0b01;
        const EFFECT = 0b10;
        const BOTH = Self::SOURCE.bits() | Self::EFFECT.bits();
    }
}
