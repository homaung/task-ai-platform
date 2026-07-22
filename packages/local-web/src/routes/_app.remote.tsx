import { createFileRoute } from '@tanstack/react-router';
import { AiRoomsPage } from '@/pages/ai-rooms/AiRoomsPage';

export const Route = createFileRoute('/_app/remote')({
  component: AiRoomsPage,
});
