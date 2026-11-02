import { setValues, type Snapshot, type Value } from "./api";
import { savedNote } from "./strings";

export type Note = { text: string; error: boolean };

/** Saves `changes`, mirrors them into `snap` on success and says what happened. */
export async function save(snap: Snapshot, changes: Record<string, Value>): Promise<Note> {
  try {
    const answer = await setValues(changes);
    Object.assign(snap.values, changes);
    return savedNote(answer);
  } catch (e) {
    return { text: String(e), error: true };
  }
}
