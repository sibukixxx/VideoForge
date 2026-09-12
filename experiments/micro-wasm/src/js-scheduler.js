export function calculateTimelineJs(input) {
  const request = JSON.parse(input);
  const gap = request.dialogue_gap_ms ?? 200;
  let cursor = 0;
  const dialogues = request.dialogues.map((dialogue, position) => {
    if (dialogue.duration_ms === 0) throw new Error(`dialogue ${dialogue.index} has zero duration`);
    if (position > 0) cursor += gap;
    const scheduled = {
      index: dialogue.index,
      speaker: dialogue.speaker,
      speaker_display: dialogue.speaker_display,
      start_ms: cursor,
      end_ms: cursor + dialogue.duration_ms,
    };
    cursor = scheduled.end_ms;
    return scheduled;
  });
  if (dialogues.length === 0) throw new Error("timeline has no dialogues");
  return JSON.stringify({ dialogues, total_duration_ms: cursor });
}
