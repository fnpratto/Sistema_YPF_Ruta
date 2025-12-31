#[macro_export]
macro_rules! election_info {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::info;
            info!("{}", format!($($arg)*).cyan().bold())
        }
    };
}

#[macro_export]
macro_rules! election_debug {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::debug;
            debug!("{}", format!($($arg)*).cyan())
        }
    };
}

#[macro_export]
macro_rules! leader_info {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::info;
            info!("{}", format!($($arg)*).green().bold())
        }
    };
}

#[macro_export]
macro_rules! leader_debug {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::debug;
            debug!("{}", format!($($arg)*).green())
        }
    };
}

#[macro_export]
macro_rules! fuel_info {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::info;
            info!("{}", format!($($arg)*).yellow())
        }
    };
}

#[macro_export]
macro_rules! fuel_debug {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::debug;
            debug!("{}", format!($($arg)*).yellow())
        }
    };
}

#[macro_export]
macro_rules! crash_info {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::info;
            info!("{}", format!($($arg)*).red().bold())
        }
    };
}

#[macro_export]
macro_rules! regional_admin_info {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::info;
            info!(target: "regional_admin", "{}", format!($($arg)*).magenta().bold())
        }
    };
}

#[macro_export]
macro_rules! regional_admin_debug {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::debug;
            debug!(target: "regional_admin", "{}", format!($($arg)*).magenta())
        }
    };
}

#[macro_export]
macro_rules! regional_admin_error {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::error;
            error!(target: "regional_admin", "{}", format!($($arg)*).red().bold())
        }
    };
}

#[macro_export]
macro_rules! company_info {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::info;
            info!("{}", format!($($arg)*).blue().bold())
        }
    };
}

#[macro_export]
macro_rules! company_debug {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::debug;
            debug!("{}", format!($($arg)*).blue())
        }
    };
}

#[macro_export]
macro_rules! company_error {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::error;
            error!("{}", format!($($arg)*).red().bold())
        }
    };
}

#[macro_export]
macro_rules! performance_info {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::info;
            info!("{}", format!($($arg)*).blue().bold())
        }
    };
}

#[macro_export]
macro_rules! performance_debug {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::debug;
            debug!("{}", format!($($arg)*).blue())
        }
    };
}

#[macro_export]
macro_rules! performance_error {
    ($($arg:tt)*) => {
        {
            use colored::Colorize;
            use log::error;
            error!("{}", format!($($arg)*).red().bold())
        }
    };
}
