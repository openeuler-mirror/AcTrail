// Defines OpenCode-to-AcTrail control protocol constants.
export const TRANSITION_TYPE = Object.freeze({
  TURN: "turn",
  WORK: "work",
  INTERACTION: "interaction",
  SESSION_CLOSED: "session_closed",
});

export const TURN_STATE = Object.freeze({
  STARTED: "started",
  COMPLETED: "completed",
});

export const INTERACTION_STATE = Object.freeze({
  REQUESTED: "requested",
  RESOLVED: "resolved",
});

export const INTERACTION_REASON = Object.freeze({
  APPROVAL: "approval",
  INPUT: "input",
});

export const CONTROL_COMMAND = Object.freeze({
  REPORT_TURN_LIFECYCLE: "report_turn_lifecycle_v2",
  REPORT_WORK_LIFECYCLE: "report_work_lifecycle",
  REPORT_USER_INTERACTION: "report_user_interaction_v2",
  REPORT_SESSION_CLOSED: "report_session_closed",
});

export const CONTROL_REPLY = Object.freeze({
  TURN_LIFECYCLE_RECORDED: "reply_turn_lifecycle_recorded",
  WORK_LIFECYCLE_RECORDED: "reply_work_lifecycle_recorded",
  USER_INTERACTION_RECORDED: "reply_user_interaction_recorded",
  SESSION_CLOSED_RECORDED: "reply_session_closed_recorded",
  ERROR: "error",
});

export const OPENCODE_EVENT = Object.freeze({
  PERMISSION_ASKED: "permission.asked",
  PERMISSION_REPLIED: "permission.replied",
  PERMISSION_V2_ASKED: "permission.v2.asked",
  PERMISSION_V2_REPLIED: "permission.v2.replied",
  QUESTION_ASKED: "question.asked",
  QUESTION_REPLIED: "question.replied",
  QUESTION_REJECTED: "question.rejected",
  QUESTION_V2_ASKED: "question.v2.asked",
  QUESTION_V2_REPLIED: "question.v2.replied",
  QUESTION_V2_REJECTED: "question.v2.rejected",
  SESSION_DELETED: "session.deleted",
  SESSION_IDLE: "session.idle",
  SESSION_STATUS: "session.status",
  MESSAGE_UPDATED: "message.updated",
  MESSAGE_PART_UPDATED: "message.part.updated",
});
