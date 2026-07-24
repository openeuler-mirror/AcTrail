use std::path::PathBuf;

use crate::elf::ElfImage;
use crate::plan::{ProbeSource, TargetIdentity, TlsProvider};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ProbeConsumer {
    PlanOnly,
    Standalone,
    Sync,
    Daemon,
}

impl ProbeConsumer {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PlanOnly => "plan-only",
            Self::Standalone => "standalone",
            Self::Sync => "sync",
            Self::Daemon => "daemon",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DetectionRequest {
    pub(crate) requested_provider: Option<TlsProvider>,
    pub(crate) requested_source: Option<ProbeSource>,
    pub(crate) libraries: Vec<PathBuf>,
    pub(crate) library_search_dirs: Vec<PathBuf>,
    pub(crate) consumer: ProbeConsumer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LibraryCandidate {
    pub(crate) path: PathBuf,
    pub(crate) note: Option<String>,
}

#[derive(Clone, Copy)]
pub(crate) struct ProbeImage<'a> {
    pub(crate) image: &'a ElfImage,
    pub(crate) source: ProbeSource,
    pub(crate) library: Option<&'a LibraryCandidate>,
}

pub(crate) struct ProbeContext<'a> {
    pub(crate) target: &'a TargetIdentity,
    pub(crate) target_image: &'a ElfImage,
    pub(crate) probe: ProbeImage<'a>,
    pub(crate) request: &'a DetectionRequest,
}

impl<'a> ProbeContext<'a> {
    pub(crate) fn executable(
        target: &'a TargetIdentity,
        image: &'a ElfImage,
        request: &'a DetectionRequest,
    ) -> Self {
        ProbeContext {
            target,
            target_image: image,
            probe: ProbeImage {
                image,
                source: ProbeSource::Executable,
                library: None,
            },
            request,
        }
    }

    pub(crate) fn for_library<'probe>(
        &'probe self,
        image: &'probe ElfImage,
        library: &'probe LibraryCandidate,
    ) -> ProbeContext<'probe>
    where
        'a: 'probe,
    {
        ProbeContext {
            target: self.target,
            target_image: self.target_image,
            probe: ProbeImage {
                image,
                source: ProbeSource::SharedLibrary,
                library: Some(library),
            },
            request: self.request,
        }
    }
}
