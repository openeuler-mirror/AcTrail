//! CLI command dispatcher.

use clap::error::ErrorKind;

use crate::capture::{
    CaptureEvent, HttpAssembler, HttpAssemblyOutput, ProbeSession, RingStatsCollector,
    SseAssembler, SseFrameEvent,
};
use crate::cli::args::{Command, try_parse_args};
use crate::cli::output::{Output, write_error};
use crate::cli::reporter::Reporter;
use crate::cli::ring_stats::RingStatsReporter;
use crate::llm_projection::LlmProjector;
use crate::{ToolError, ToolResult};

pub(crate) fn run_from_env() -> ToolResult<()> {
    run_command(try_parse_args().map_err(cli_parse_error)?)
}

pub(crate) fn main_from_env() -> i32 {
    match try_parse_args() {
        Ok(command) => match run_command(command) {
            Ok(()) => 0,
            Err(error) => {
                let _ = write_error(&error);
                1
            }
        },
        Err(error) => {
            let exit_code = cli_exit_code(&error);
            let text = error.to_string();
            let result = if exit_code == 0 {
                Output::stdout(&text)
            } else {
                Output::stderr(&text)
            };
            let _ = result;
            exit_code
        }
    }
}

fn run_command(command: Command) -> ToolResult<()> {
    match command {
        Command::Probe(args) => {
            let reporter_config = args.reporter_config();
            let config = args.into_config()?;
            let mut assembler = HttpAssembler::new(&config);
            let mut sse_assembler = SseAssembler::new(&config);
            let mut llm_projector = LlmProjector::new(&config);
            let mut reporter = Reporter::new(reporter_config);
            let mut ring_stats = reporter_config.ring_stats.then(RingStatsCollector::default);
            let result = ProbeSession::run(config, |event: CaptureEvent| {
                if let Some(stats) = &mut ring_stats {
                    stats.observe(&event);
                }
                reporter.event(&event)?;
                for output in assembler.push(&event)? {
                    report_assembly_output(
                        &mut reporter,
                        &mut sse_assembler,
                        &mut llm_projector,
                        output,
                    )?;
                }
                Ok(())
            })?;
            for output in assembler.finish()? {
                report_assembly_output(
                    &mut reporter,
                    &mut sse_assembler,
                    &mut llm_projector,
                    output,
                )?;
            }
            for output in sse_assembler.finish()? {
                report_sse_output(&mut reporter, &mut llm_projector, output)?;
            }
            for output in llm_projector.finish() {
                reporter.llm_output(&output)?;
            }
            reporter.target_exit(result.status)?;
            if let Some(stats) = ring_stats {
                RingStatsReporter::new().ring_stats(&stats.finish(result.ring_lost_stats))?;
            }
            Ok(())
        }
    }
}

fn report_assembly_output(
    reporter: &mut Reporter,
    sse_assembler: &mut SseAssembler,
    llm_projector: &mut LlmProjector,
    output: HttpAssemblyOutput,
) -> ToolResult<()> {
    match output {
        HttpAssemblyOutput::Message(message) => {
            reporter.http_message(&message)?;
            for llm_output in llm_projector.push_http_message(&message)? {
                reporter.llm_output(&llm_output)?;
            }
            report_sse_outputs(
                reporter,
                llm_projector,
                sse_assembler.push_message(&message)?,
            )
        }
        HttpAssemblyOutput::BodyFragment(fragment) => {
            reporter.http_body_fragment(&fragment)?;
            for llm_output in llm_projector.push_http_fragment(&fragment)? {
                reporter.llm_output(&llm_output)?;
            }
            report_sse_outputs(
                reporter,
                llm_projector,
                sse_assembler.push_fragment(&fragment)?,
            )
        }
    }
}

fn report_sse_outputs(
    reporter: &mut Reporter,
    llm_projector: &mut LlmProjector,
    outputs: Vec<SseFrameEvent>,
) -> ToolResult<()> {
    for output in outputs {
        report_sse_output(reporter, llm_projector, output)?;
    }
    Ok(())
}

fn report_sse_output(
    reporter: &mut Reporter,
    llm_projector: &mut LlmProjector,
    output: SseFrameEvent,
) -> ToolResult<()> {
    reporter.sse_frame(&output.frame)?;
    for llm_output in llm_projector.push_frame(&output.frame, &output.data) {
        reporter.llm_output(&llm_output)?;
    }
    Ok(())
}

fn cli_parse_error(error: clap::Error) -> ToolError {
    ToolError::new(error.to_string())
}

fn cli_exit_code(error: &clap::Error) -> i32 {
    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
        _ => 2,
    }
}
