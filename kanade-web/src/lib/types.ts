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
  output_ids: string[];
  queue: Track[];
  current_index: number | null;
  status: PlaybackStatus;
  position_secs: number;
  volume: number;
  shuffle: boolean;
  repeat: RepeatMode;
}

// WS Protocol
export type WsCommand =
  | { cmd: "play"; node_id: string }
  | { cmd: "pause"; node_id: string }
  | { cmd: "stop"; node_id: string }
  | { cmd: "next"; node_id: string }
  | { cmd: "previous"; node_id: string }
  | { cmd: "seek"; node_id: string; position_secs: number }
  | { cmd: "set_volume"; node_id: string; volume: number }
  | { cmd: "set_repeat"; node_id: string; repeat: RepeatMode }
  | { cmd: "set_shuffle"; node_id: string; shuffle: boolean }
  | { cmd: "add_to_queue"; node_id: string; track: Track }
  | { cmd: "add_tracks_to_queue"; node_id: string; tracks: Track[] }
  | { cmd: "play_index"; node_id: string; index: number }
  | { cmd: "remove_from_queue"; node_id: string; index: number }
  | { cmd: "move_in_queue"; node_id: string; from: number; to: number }
  | { cmd: "clear_queue"; node_id: string }
  | { cmd: "replace_and_play"; node_id: string; tracks: Track[]; index: number };

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
  | { req: "get_queue"; node_id: string };

export type ClientMessage =
  | WsCommand
  | ({ req_id: number } & WsRequest);

export type ServerMessage =
  | { type: "state"; state: { nodes: Node[] } }
  | { type: "response"; req_id: number; data: WsResponse };

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
  | { queue: { tracks: Track[]; current_index: number | null } };
