#[derive(Debug)]
#[repr(isize)]
enum SbiErrorType {
    Success,
    Failed,
    NotSupported,
    InvalidParam,
    Denied,
    InvalidAddress,
    AlreadyAvailable,
    AlreadyStarted,
    AlreadyStopped,
    NoSharedMemory,
    InvalidState,
    BadRange,
    Timeout,
    Io,
    DeniedLocked,
}

impl From<isize> for SbiErrorType {
    fn from(value: isize) -> Self {
        match value {
            0 => SbiErrorType::Success,
            -1 => SbiErrorType::Failed,
            -2 => SbiErrorType::NotSupported,
            -3 => SbiErrorType::InvalidParam,
            -4 => SbiErrorType::Denied,
            -5 => SbiErrorType::InvalidAddress,
            -6 => SbiErrorType::AlreadyAvailable,
            -7 => SbiErrorType::AlreadyStarted,
            -8 => SbiErrorType::AlreadyStopped,
            -9 => SbiErrorType::NoSharedMemory,
            -10 => SbiErrorType::InvalidState,
            -11 => SbiErrorType::BadRange,
            -12 => SbiErrorType::Timeout,
            -13 => SbiErrorType::Io,
            -14 => SbiErrorType::DeniedLocked,
            unexpected => panic!("Unexpected SbiErrorType: {unexpected}"),
        }
    }
}
