"""
Project Management
Represents a single project with its tasks stored in a text file
"""

import logging
from pathlib import Path
from typing import Any, Dict, List, Optional

from textrec.text_records import TextRecords
from .task import Task

logger = logging.getLogger(__name__)

class Project:
    """Represents a single project with its tasks.

    ToDo tasks are stored in ``file_path`` (e.g. ``pjpd/tasks.txt``) and Done
    tasks are stored in ``done_file_path`` (e.g. ``pjpd/tasks_done.txt``).
    The done file is written without a ``bak/`` backup since it acts as an
    archive that grows over time.
    """

    def __init__(self, name: str, file_path: Path, done_file_path: Path):
        self.name = name
        self.file_path = Path(file_path)
        self.done_file_path = Path(done_file_path)
        self.text_records = TextRecords(self.file_path.parent)
        self._tasks: Optional[List[Task]] = None

    @property
    def tasks(self) -> List[Task]:
        """Get all tasks in this project, loading from file if needed"""
        if self._tasks is None:
            self._load_tasks()
        return self._tasks

    def _load_tasks(self) -> None:
        """Load tasks from both the todo and done project files."""
        self._tasks = [
            task
            for path in (self.file_path, self.done_file_path)
            for record_text in self.text_records.read_records(path)
            if (task := Task.from_text(record_text)) is not None
        ]
    
    def add_task(self, description: str, priority: int, tag: str) -> Task:
        """Add a new task to this project"""
        # Generate a unique task ID using the provided tag
        task = Task(
            id=Task.generate_task_id(tag),
            tag=tag,
            priority=priority,
            status="ToDo",
            description=description,
        )
        task.stamp_created()

        self.tasks.append(task)
        self._save_tasks()
        return task
    
    def get_task(self, task_id: str) -> Optional[Task]:
        """Get a task by its ID"""
        for task in self.tasks:
            if task.id == task_id:
                return task
        return None
    
    def update_task(self, task_id: str, description: Optional[str],
                   priority: Optional[int], status: Optional[str]) -> Optional[Task]:
        """Update an existing task"""
        task = self.get_task(task_id)
        if not task:
            return None
            
        if description is not None:
            task.description = description
        if priority is not None:
            task.priority = priority
        if status is not None:
            task.status = status

        task.stamp_updated()
        self._save_tasks()
        return task
    
    def mark_done(self, task_id: str) -> Optional[Task]:
        """Mark a task as completed"""
        return self.update_task(task_id, description=None, priority=None, status="Done")
    
    def _save_tasks(self) -> None:
        """Save todo tasks to ``file_path`` (with backup) and done tasks to ``done_file_path`` (no backup)."""
        try:
            by_priority = lambda t: -t.priority
            todo_tasks = sorted((t for t in self.tasks if t.status != "Done"), key=by_priority)
            done_tasks = sorted((t for t in self.tasks if t.status == "Done"), key=by_priority)

            todo_content = self.text_records.join_records(t.to_text() for t in todo_tasks)
            self.text_records.write_atomic(self.file_path, todo_content)

            if done_tasks or self.done_file_path.exists():
                done_content = self.text_records.join_records(t.to_text() for t in done_tasks)
                self.text_records.write_atomic_no_backup(self.done_file_path, done_content)

        except Exception as e:
            logger.error(f"Error saving tasks for project {self.name}: {e}")
            raise
    
    def get_task_count(self) -> int:
        """Get the total number of tasks"""
        return len(self.tasks)
    
    def get_tasks_by_priority(self, priority: int) -> List[Task]:
        """Get all tasks with priority >= the specified value"""
        return [task for task in self.tasks if task.priority >= priority]
    
    def get_tasks_by_status(self, status: str) -> List[Task]:
        """Get all tasks with a specific status"""
        return [task for task in self.tasks if task.status == status]
    
    def get_overview(self) -> Dict[str, Any]:
        """Get an overview of this project"""
        todo_tasks = self.get_tasks_by_status("ToDo")
        done_tasks = self.get_tasks_by_status("Done")
        
        high_priority_todo = [t for t in todo_tasks if t.priority >= 100]
        medium_priority_todo = [t for t in todo_tasks if 10 <= t.priority < 100]
        low_priority_todo = [t for t in todo_tasks if t.priority < 10]
        
        return {
            "name": self.name,
            "total_tasks": len(self.tasks),
            "todo_tasks": len(todo_tasks),
            "done_tasks": len(done_tasks),
            "high_priority_todo": len(high_priority_todo),
            "medium_priority_todo": len(medium_priority_todo),
            "low_priority_todo": len(low_priority_todo)
        }
    
    def filter_tasks(self, status: Optional[str]) -> List[Dict[str, Any]]:
        """Filter tasks by status, returning them as dictionaries.

        Args:
            status: Filter tasks by status ("ToDo" or "Done"). None returns all tasks.

        Returns:
            List of task dictionaries with id, priority, status, and description.
        """
        filtered_tasks = []

        for task in self.tasks:
            # Filter by status if specified (case-insensitive comparison)
            if status is not None and task.status.lower() != status.lower():
                continue
                
            filtered_tasks.append({
                "id": task.id,
                "priority": task.priority,
                "status": task.status,
                "description": task.description,
            })
            
        return filtered_tasks 