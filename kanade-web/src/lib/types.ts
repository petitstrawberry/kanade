// Core models
export interface Track {
  id: string;
  file_path: string;
  album_id: string | null;
  title: string | null;
  artist: string | null;
  album_artist: string | null;
  album_title: string | null;
  composer: string | null;
  genre: string | null;
  track_number: number | null;
  disc_number: number | null;
  duration_secs: number | null;
  format: string | null;
  sample_rate: number | null;
}

export interface Album {
  id: string;
  dir_path: string;
  title: string | null;
  artwork_path: string | null;
}

export type RepeatMode = "off" | "one" | "all";
export type PlaybackStatus = "stopped" | "playing" | "paused" | "loading";

export interface Node {
  id: string;
  name: string;
  connected: boolean;
  status: PlaybackStatus;
  position_secs: number;
  volume: number;
}

export interface PlaybackState {
  nodes: Node[];
  selected_node_id: string | null;
  queue: Track[];
  current_index: number | null;
  shuffle: boolean;
  repeat: RepeatMode;
}

// WS Protocol
export type WsCommand =
  | { cmd: "play" }
  | { cmd: "pause" }
  | { cmd: "stop" }
  | { cmd: "next" }
  | { cmd: "previous" }
  | { cmd: "seek"; position_secs: number }
  | { cmd: "set_volume"; volume: number }
  | { cmd: "set_repeat"; repeat: RepeatMode }
  | { cmd: "set_shuffle"; shuffle: boolean }
  | { cmd: "add_to_queue"; track: Track }
  | { cmd: "add_tracks_to_queue"; tracks: Track[] }
  | { cmd: "play_index"; index: number }
  | { cmd: "remove_from_queue"; index: number }
  | { cmd: "move_in_queue"; from: number; to: number }
  | { cmd: "clear_queue" }
  | { cmd: "replace_and_play"; tracks: Track[]; index: number }
  | { cmd: "select_node"; node_id: string };

export type WsRequest =
  | { req: "get_albums" }
  | { req: "get_album_tracks"; album_id: string }
  | { req: "get_artists" }
  | { req: "get_artist_albums"; artist: string }
  | { req: "get_artist_tracks"; artist: string }
  | { req: "get_genres" }
  | { req: "get_genre_albums"; genre: string }
  | { req: "get_genre_tracks"; genre: string }
  | { req: "search"; query: string }
  | { req: "get_queue" }
  | { req: "sign_urls"; paths: string[] };

export type ClientMessage =
  | WsCommand
  | ({ req_id: number } & WsRequest);

export type ServerMessage =
  | { type: "state"; state: PlaybackState }
  | { type: "response"; req_id: number; data: WsResponse }
  | { type: "media_auth"; media_auth_key: string; media_auth_key_id: string };

export type WsResponse =
  | { albums: Album[] }
  | { album_tracks: Track[] }
  | { artists: string[] }
  | { artist_albums: Album[] }
  | { artist_tracks: Track[] }
  | { genres: string[] }
  | { genre_albums: Album[] }
  | { genre_tracks: Track[] }
  | { search_results: Track[] }
  | { queue: { tracks: Track[]; current_index: number | null } }
  | { signed_urls: Record<string, string> };
