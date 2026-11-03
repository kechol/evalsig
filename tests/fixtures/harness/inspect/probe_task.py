from inspect_ai import Task, task
from inspect_ai.dataset import Sample
from inspect_ai.scorer import includes
from inspect_ai.solver import generate


@task
def probe():
    return Task(
        dataset=[
            Sample(id="s1", input="Say yes", target="yes", metadata={"stratum": "a"}),
            Sample(id="s2", input="Say no", target="Default", metadata={"stratum": "b"}),
        ],
        solver=generate(),
        scorer=includes(),
        epochs=2,
    )
