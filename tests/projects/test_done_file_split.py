"""
Tests that ToDo tasks live in tasks.txt and Done tasks live in tasks_done.txt,
and that tasks_done.txt is not backed up to bak/.
"""

import pytest

from src.projects.projects import Projects


class TestDoneFileSplit:
    @pytest.fixture
    def projects_manager(self, tmp_path):
        return Projects(tmp_path)

    def test_todo_task_only_in_tasks_file(self, projects_manager):
        task = projects_manager.add_task("A todo", 50, "todo")

        assert projects_manager.tasks_file.exists()
        assert not projects_manager.done_tasks_file.exists()

        content = projects_manager.tasks_file.read_text(encoding="utf-8")
        assert task.id in content

    def test_marking_done_moves_task_to_done_file(self, projects_manager):
        task = projects_manager.add_task("Finish me", 50, "todo")
        projects_manager.mark_task_done(task.id)

        todo_content = projects_manager.tasks_file.read_text(encoding="utf-8")
        done_content = projects_manager.done_tasks_file.read_text(encoding="utf-8")

        assert task.id not in todo_content
        assert task.id in done_content
        assert "Status: Done" in done_content

    def test_done_file_is_not_backed_up(self, projects_manager):
        task = projects_manager.add_task("Finish me", 50, "todo")
        projects_manager.mark_task_done(task.id)

        bak_dir = projects_manager.tasks_file.parent / "bak"
        # bak/ should exist for tasks.txt history but contain no tasks_done.* files
        if bak_dir.exists():
            done_backups = [p for p in bak_dir.iterdir() if p.name.startswith("tasks_done.")]
            assert done_backups == []

    def test_marking_done_then_reloading_keeps_task_visible(self, projects_manager):
        task = projects_manager.add_task("Finish me", 50, "todo")
        projects_manager.mark_task_done(task.id)

        fresh = Projects(projects_manager.project_dir)
        reloaded = fresh.get_task(task.id)
        assert reloaded is not None
        assert reloaded.status == "Done"
        assert reloaded.priority == 50

    def test_legacy_mixed_file_migrates_on_save(self, projects_manager):
        """A pre-existing tasks.txt with both ToDo and Done splits on next save."""
        projects_manager._ensure_project()
        projects_manager.tasks_file.write_text(
            "Priority:   50\nStatus: ToDo\nID: keep-aaaa\nKeep me\n"
            "----\n"
            "Priority:   50\nStatus: Done\nID: move-bbbb\nMove me",
            encoding="utf-8",
        )

        # Trigger a save by adding any new task
        projects_manager.add_task("Trigger save", 10, "trig")

        todo_content = projects_manager.tasks_file.read_text(encoding="utf-8")
        done_content = projects_manager.done_tasks_file.read_text(encoding="utf-8")

        assert "keep-aaaa" in todo_content
        assert "move-bbbb" not in todo_content
        assert "move-bbbb" in done_content
