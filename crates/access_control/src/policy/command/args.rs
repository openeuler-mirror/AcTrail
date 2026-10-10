use std::cmp::Ordering;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CommandArgsPattern {
    Any,
    Exact(Vec<String>),
    Prefix(Vec<String>),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum CommandArgsScope {
    Any,
    Exact(Vec<String>),
    Prefix(Vec<String>),
}

impl CommandArgsPattern {
    pub(super) fn parse(args: Option<&[String]>) -> Result<Self, String> {
        let Some(args) = args else {
            return Ok(Self::Any);
        };
        for (index, arg) in args.iter().enumerate() {
            if arg.contains('\0') {
                return Err(format!("command rule args[{index}] contains NUL"));
            }
            if arg == "*" && index + 1 != args.len() {
                return Err(
                    "command rule args wildcard * is only allowed as the final item".to_string(),
                );
            }
        }
        if args.last().is_some_and(|arg| arg == "*") {
            return Ok(Self::Prefix(args[..args.len() - 1].to_vec()));
        }
        Ok(Self::Exact(args.to_vec()))
    }

    pub(super) fn matches(&self, args: &[String]) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(expected) => expected == args,
            Self::Prefix(prefix) => args.starts_with(prefix),
        }
    }

    pub(super) fn requires_snapshot(&self) -> bool {
        match self {
            Self::Any => false,
            Self::Prefix(prefix) => !prefix.is_empty(),
            Self::Exact(_) => true,
        }
    }

    pub(super) fn view(&self) -> Option<Vec<String>> {
        match self {
            Self::Any => None,
            Self::Exact(args) => Some(args.clone()),
            Self::Prefix(prefix) => {
                let mut args = prefix.clone();
                args.push("*".to_string());
                Some(args)
            }
        }
    }

    pub(super) fn logical_scope(&self) -> CommandArgsScope {
        match self {
            Self::Any => CommandArgsScope::Any,
            Self::Prefix(prefix) if prefix.is_empty() => CommandArgsScope::Any,
            Self::Exact(args) => CommandArgsScope::Exact(args.clone()),
            Self::Prefix(prefix) => CommandArgsScope::Prefix(prefix.clone()),
        }
    }

    pub(super) fn describe(&self) -> String {
        self.view().map_or_else(
            || "<any>".to_string(),
            |args| {
                serde_json::to_string(&args)
                    .expect("command args string vector serialization cannot fail")
            },
        )
    }
}

impl Ord for CommandArgsPattern {
    fn cmp(&self, other: &Self) -> Ordering {
        self.logical_scope().cmp(&other.logical_scope())
    }
}

impl PartialOrd for CommandArgsPattern {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
