export type AtlasTone = 'neutral' | 'active' | 'ai' | 'verified' | 'attention' | 'error';

export type AtlasNode = {
  id: string;
  label: string;
  detail?: string;
  tone: AtlasTone;
};

export type AtlasEdge = {
  source: string;
  target: string;
  label?: string;
};

export type AtlasModel = {
  title: string;
  description: string;
  nodes: AtlasNode[];
  edges: AtlasEdge[];
  selectedId?: string;
};
