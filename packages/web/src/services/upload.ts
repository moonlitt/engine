const API_BASE = 'http://localhost:3001';

/**
 * Upload a MIDI file to the server for a specific track.
 *
 * The server stores the file, adds a clip to the track, and broadcasts
 * a `midi.clip_added` event over WebSocket so all clients update.
 */
export async function uploadMidiFile(file: File, trackId: number): Promise<boolean> {
  const formData = new FormData();
  formData.append('file', file);
  formData.append('trackId', trackId.toString());

  try {
    const res = await fetch(`${API_BASE}/api/upload-midi`, {
      method: 'POST',
      body: formData,
    });

    if (!res.ok) {
      const body = await res.json().catch(() => ({ error: 'Upload failed' }));
      console.error('[upload] MIDI upload failed:', body.error);
      return false;
    }

    return true;
  } catch (err) {
    console.error('[upload] MIDI upload error:', err);
    return false;
  }
}
