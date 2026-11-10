use dstudio_core::assistant::*;
use dstudio_core::*;

fn doc() -> Document {
    Document::new(Project::default())
}

#[test]
fn assist_mode_executes_validated_edits_as_one_undo_step() {
    let mut d = doc();
    let audio = d.project().tracks[0].id.0;
    let raw = format!(
        r#"{{"actions":[
        {{"type":"edit","command":{{"op":"add_track","index":null,"track":{{"id":0,"kind":"audio","name":"Drums","color":2,"gain":1.0,"pan":0.0,"mute":false,"solo":false,"clips":[]}}}}}},
        {{"type":"set_track_volume_db","track":{audio},"db":-6.0}},
        {{"type":"edit","command":{{"op":"set_tempo","bpm":128.0}}}},
        {{"type":"explain","text":"Added drums and turned the audio track down."}}
    ]}}"#
    );
    let turn = parse_turn(&raw).unwrap();
    let plan = plan(&d, Mode::Assist, &turn).unwrap();
    assert_eq!(plan.summary, vec!["Added an audio track, Drums.", "Set Audio volume to -6.0 dB.", "Changed tempo to 128 BPM."]);
    assert_eq!(d.project().tracks.len(), 2, "planning must not modify the document");

    commit(&mut d, &plan).unwrap();
    assert_eq!(d.project().tracks.len(), 3);
    assert_eq!(d.project().tracks[2].name, "Drums");
    assert_eq!(d.project().transport.bpm, 128.0);
    assert_eq!(d.undo_label(), Some("Assistant Edit"));
    d.undo().unwrap();
    assert_eq!(d.project().tracks.len(), 2);
    assert_eq!(d.project().transport.bpm, 120.0);
    assert_eq!(d.project().tracks[0].gain, 1.0);
}

#[test]
fn teach_mode_rejects_any_edit_but_allows_highlights() {
    let d = doc();
    let t = d.project().tracks[0].id.0;
    let ok = parse_turn(&format!(
        r#"{{"actions":[{{"type":"highlight","target":"transport.bpm","note":"Double-click to type a tempo"}},
                        {{"type":"highlight","target":"track:{t}:volume","note":"Drag to set level"}},
                        {{"type":"explain","text":"Tempo lives in the transport bar."}}]}}"#
    ))
    .unwrap();
    let p = plan(&d, Mode::Teach, &ok).unwrap();
    assert!(p.batch.is_none());
    assert_eq!(p.highlights.len(), 2);

    let bad = parse_turn(r#"{"actions":[{"type":"explain","text":"x"},{"type":"edit","command":{"op":"set_tempo","bpm":90.0}}]}"#).unwrap();
    assert_eq!(plan(&d, Mode::Teach, &bad), Err(AssistantError::EditInTeachMode { index: 1 }));
}

#[test]
fn invalid_targets_and_edits_are_rejected_before_execution() {
    let d = doc();
    let t = parse_turn(r#"{"actions":[{"type":"highlight","target":"window.self_destruct","note":""}]}"#).unwrap();
    assert!(matches!(plan(&d, Mode::Assist, &t), Err(AssistantError::UnknownTarget(_))));
    let t = parse_turn(r#"{"actions":[{"type":"highlight","target":"track:999:volume","note":""}]}"#).unwrap();
    assert!(matches!(plan(&d, Mode::Assist, &t), Err(AssistantError::UnknownTarget(_))));
    let t = parse_turn(r#"{"actions":[{"type":"edit","command":{"op":"set_tempo","bpm":90.0}},{"type":"edit","command":{"op":"set_tempo","bpm":5000.0}}]}"#).unwrap();
    assert!(matches!(plan(&d, Mode::Assist, &t), Err(AssistantError::InvalidEdit { index: 1, .. })));
    assert!(matches!(parse_turn("not json"), Err(AssistantError::Malformed(_))));
}

#[test]
fn later_edits_may_reference_tracks_created_earlier_in_the_turn() {
    let mut d = doc();
    // The model can't know the new id, so it creates with an explicit id.
    let next = d.project().next_id;
    let raw = format!(
        r#"{{"actions":[
        {{"type":"edit","command":{{"op":"add_track","index":null,"track":{{"id":{next},"kind":"audio","name":"Vox","color":3,"gain":1.0,"pan":0.0,"mute":false,"solo":false,"clips":[]}}}}}},
        {{"type":"edit","command":{{"op":"set_track_pan","track":{next},"pan":-0.25}}}}]}}"#
    );
    let p = plan(&d, Mode::Assist, &parse_turn(&raw).unwrap()).unwrap();
    assert_eq!(p.summary[1], "Panned Vox 25% left.");
    commit(&mut d, &p).unwrap();
    assert_eq!(d.project().track(TrackId(next)).unwrap().pan, -0.25);
}

#[test]
fn state_description_and_schema_are_well_formed() {
    let d = doc();
    let s = describe_state(d.project());
    assert_eq!(s["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(s["tracks"][0]["volume_db"], 0.0);
    assert_eq!(s["tempo_bpm"], 120.0);
    let schema = tool_schema();
    assert_eq!(schema["properties"]["actions"]["items"]["oneOf"].as_array().unwrap().len(), 4);
}
